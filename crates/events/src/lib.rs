//! Recoverable event distribution. SQL is authoritative; PubAck and consumer
//! acknowledgements are explicit external boundaries and never decide commit.
use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, PullConsumer},
    message::{AckKind, PublishMessage},
};
use celetixo_engine::ids::{Digest, Scope};
use celetixo_postgres::delivery::{DeliveryClaim, DeliveryError, EventDelivery, QuarantineReason};
use celetixo_protocol::MAX_WIRE_BYTES;
use futures_util::StreamExt;
use std::{path::PathBuf, sync::Arc, time::Duration};
use thiserror::Error;
pub mod worker;

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_DELIVER: i64 = 8;
const BATCH: usize = 8;

/// Trusted operator configuration. Private keys remain files in the secret
/// boundary; no credentials or server error payloads are printed by this crate.
pub struct BrokerConfiguration {
    /// Explicit operator binding of the authenticated NATS account/subject to
    /// this SQL scope. Account/subject ACLs are independently enforced by NATS.
    pub scope: Scope,
    pub servers: Vec<String>,
    pub ca_file: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    pub subject: String,
    pub stream: String,
    pub consumer: String,
    pub inbox_prefix: String,
}

#[derive(Debug, Error)]
pub enum EventError {
    #[error("invalid bounded broker configuration")]
    Configuration,
    #[error("broker and database event scopes differ")]
    Scope,
    #[error("broker operation unavailable at {0}")]
    Broker(&'static str),
    #[error("broker operation exceeded its deadline")]
    Deadline,
    #[error("database delivery task failed")]
    DatabaseTask,
    #[error(transparent)]
    Database(#[from] DeliveryError),
}
type Result<T> = std::result::Result<T, EventError>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BatchReport {
    pub published: u32,
    pub applied: u32,
    pub replayed: u32,
    pub quarantined: u32,
    pub deferred: u32,
}

pub struct NatsDelivery {
    client: async_nats::Client,
    jetstream: jetstream::Context,
    configuration: BrokerConfiguration,
    consumer_stream: tokio::sync::Mutex<Option<jetstream::consumer::pull::Stream>>,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
}
fn subject(value: &str) -> bool {
    value.len() <= 1024 && value.split('.').all(token)
}

impl NatsDelivery {
    pub async fn connect(configuration: BrokerConfiguration) -> Result<Self> {
        if configuration.servers.is_empty()
            || configuration.servers.len() > 8
            || configuration.servers.iter().any(|value| {
                !value.starts_with("tls://")
                    || value.len() > 2048
                    || value.contains(['@', '?', '#'])
            })
            || !subject(&configuration.subject)
            || !token(&configuration.stream)
            || !token(&configuration.consumer)
            || !subject(&configuration.inbox_prefix)
            || !configuration.inbox_prefix.starts_with("_INBOX.")
            || [
                &configuration.ca_file,
                &configuration.certificate,
                &configuration.private_key,
            ]
            .iter()
            .any(|path| !path.is_absolute())
        {
            return Err(EventError::Configuration);
        }
        let client = async_nats::ConnectOptions::new()
            .add_root_certificates(configuration.ca_file.clone())
            .add_client_certificate(
                configuration.certificate.clone(),
                configuration.private_key.clone(),
            )
            .require_tls(true)
            .tls_first()
            .connection_timeout(IO_TIMEOUT)
            .request_timeout(Some(IO_TIMEOUT))
            .max_reconnects(8)
            .subscription_capacity(64)
            .client_capacity(64)
            .custom_inbox_prefix(configuration.inbox_prefix.clone())
            .connect(configuration.servers.clone())
            .await
            .map_err(|_| EventError::Broker("connect"))?;
        if client.server_info().max_payload > MAX_WIRE_BYTES {
            return Err(EventError::Configuration);
        }
        let mut jetstream = jetstream::new(client.clone());
        jetstream.set_timeout(IO_TIMEOUT);
        Ok(Self {
            client,
            jetstream,
            configuration,
            consumer_stream: tokio::sync::Mutex::new(None),
        })
    }

    /// No durable delivery marker is written here. Callers must first obtain
    /// this PubAck, then conditionally finish the exact live SQL claim.
    pub async fn publish_claim(&self, claim: &DeliveryClaim) -> Result<(String, u64)> {
        let scope = claim
            .event()
            .event()
            .scope
            .as_ref()
            .ok_or(EventError::Scope)?;
        if scope.organization_id != self.configuration.scope.organization.as_str()
            || scope.workspace_id != self.configuration.scope.workspace.as_str()
            || scope.repository_id != self.configuration.scope.repository.as_str()
        {
            return Err(EventError::Scope);
        }
        let id = Digest::from_bytes(
            claim
                .event()
                .event()
                .event_id
                .clone()
                .try_into()
                .map_err(|_| EventError::Configuration)?,
        );
        let message = PublishMessage::build()
            .payload(claim.bytes().to_vec().into())
            .message_id(id.to_string())
            .expected_stream(&self.configuration.stream);
        let ack = tokio::time::timeout(IO_TIMEOUT, async {
            self.jetstream
                .send_publish(self.configuration.subject.clone(), message)
                .await
                .map_err(|_| EventError::Broker("publish_send"))?
                .await
                .map_err(|_| EventError::Broker("publish_ack"))
        })
        .await
        .map_err(|_| EventError::Deadline)??;
        if ack.stream != self.configuration.stream || ack.sequence == 0 {
            return Err(EventError::Broker("publish_binding"));
        }
        Ok((ack.stream, ack.sequence))
    }

    pub async fn publish_once(&self, store: Arc<EventDelivery>) -> Result<BatchReport> {
        if store.scope() != &self.configuration.scope {
            return Err(EventError::Scope);
        }
        let database = store.clone();
        let claims = tokio::task::spawn_blocking(move || {
            database.claim(BATCH as u32, Duration::from_secs(60), 8)
        })
        .await
        .map_err(|_| EventError::DatabaseTask)??;
        let mut report = BatchReport::default();
        for claim in claims {
            let outcome = self.publish_claim(&claim).await;
            let database = store.clone();
            let confirmed = outcome.is_ok();
            tokio::task::spawn_blocking(move || match outcome {
                Ok((stream, sequence)) => database.confirm_published(&claim, &stream, sequence),
                Err(_) => database.retry_later(
                    &claim,
                    Duration::from_secs(1_u64 << claim.attempts().min(8)),
                ),
            })
            .await
            .map_err(|_| EventError::DatabaseTask)??;
            if confirmed {
                report.published += 1;
            } else {
                report.deferred += 1;
            }
        }
        Ok(report)
    }

    async fn consumer(&self) -> Result<PullConsumer> {
        let consumer: PullConsumer = self
            .jetstream
            .get_consumer_from_stream(&self.configuration.consumer, &self.configuration.stream)
            .await
            .map_err(|_| EventError::Broker("consumer_info"))?;
        let config = &consumer.cached_info().config;
        if config.ack_policy != AckPolicy::Explicit
            || config.max_deliver != MAX_DELIVER
            || config.filter_subject != self.configuration.subject
            || config.max_ack_pending > 64
            || config.max_ack_pending <= 0
            || config.ack_wait > Duration::from_secs(60)
            || config.ack_wait < Duration::from_secs(1)
            || config.durable_name.as_deref() != Some(&self.configuration.consumer)
        {
            return Err(EventError::Configuration);
        }
        Ok(consumer)
    }

    pub async fn consume_once(&self, store: Arc<EventDelivery>) -> Result<BatchReport> {
        if store.scope() != &self.configuration.scope {
            return Err(EventError::Scope);
        }
        let consumer = self.consumer().await?;
        // The continuous stream API handles JetStream's normal 409 "Batch
        // Completed" control status. The older fetch Batch API does not. Pull
        // size, bytes, pending acknowledgements and per-call work remain bounded.
        let mut stream = self.consumer_stream.lock().await;
        if stream.is_none() {
            *stream = Some(
                consumer
                    .stream()
                    .max_messages_per_batch(BATCH)
                    .max_bytes_per_batch(BATCH * MAX_WIRE_BYTES)
                    .expires(Duration::from_secs(1))
                    .messages()
                    .await
                    .map_err(|_| EventError::Broker("consumer_batch"))?,
            );
        }
        let messages = stream
            .as_mut()
            .ok_or(EventError::Broker("consumer_stream"))?;
        let mut report = BatchReport::default();
        for _ in 0..BATCH {
            let Ok(Some(message)) =
                tokio::time::timeout(Duration::from_secs(1), messages.next()).await
            else {
                break;
            };
            let message = message.map_err(|_| EventError::Broker("consumer_message"))?;
            let metadata = message
                .info()
                .map_err(|_| EventError::Broker("consumer_metadata"))?;
            if metadata.stream != self.configuration.stream
                || metadata.consumer != self.configuration.consumer
                || message.subject.as_str() != self.configuration.subject
            {
                return Err(EventError::Broker("consumer_binding"));
            }
            let database = store.clone();
            let bytes = message.payload.to_vec();
            let result = tokio::task::spawn_blocking(move || database.apply(&bytes))
                .await
                .map_err(|_| EventError::DatabaseTask)?;
            match result {
                Ok(receipt) => {
                    // A lost acknowledgement causes a later inbox replay.
                    message
                        .double_ack()
                        .await
                        .map_err(|_| EventError::Broker("consumer_ack"))?;
                    if receipt.replayed {
                        report.replayed += 1;
                    } else {
                        report.applied += 1;
                    }
                }
                Err(error) => {
                    let reason = match error {
                        DeliveryError::Schema => Some(QuarantineReason::Schema),
                        DeliveryError::Scope => Some(QuarantineReason::Scope),
                        DeliveryError::Authority => Some(QuarantineReason::Authority),
                        DeliveryError::Substitution => Some(QuarantineReason::Substitution),
                        _ => None,
                    };
                    if let Some(reason) = reason {
                        let database = store.clone();
                        let bytes = message.payload.to_vec();
                        tokio::task::spawn_blocking(move || database.quarantine(&bytes, reason))
                            .await
                            .map_err(|_| EventError::DatabaseTask)??;
                        message
                            .double_ack_with(AckKind::Term)
                            .await
                            .map_err(|_| EventError::Broker("quarantine_ack"))?;
                        report.quarantined += 1;
                    } else {
                        message
                            .ack_with(AckKind::Nak(Some(Duration::from_secs(1))))
                            .await
                            .map_err(|_| EventError::Broker("consumer_nak"))?;
                        report.deferred += 1;
                    }
                }
            }
        }
        Ok(report)
    }

    pub async fn reconcile_once(&self, store: Arc<EventDelivery>) -> Result<BatchReport> {
        if store.scope() != &self.configuration.scope {
            return Err(EventError::Scope);
        }
        let database = store.clone();
        let frames = tokio::task::spawn_blocking(move || database.missing_frames(BATCH as u32))
            .await
            .map_err(|_| EventError::DatabaseTask)??;
        let mut report = BatchReport::default();
        for bytes in frames {
            let database = store.clone();
            let receipt = tokio::task::spawn_blocking(move || database.apply(&bytes))
                .await
                .map_err(|_| EventError::DatabaseTask)??;
            if receipt.replayed {
                report.replayed += 1;
            } else {
                report.applied += 1;
            }
        }
        Ok(report)
    }

    /// Bounded graceful drain. It does not acknowledge any unfinished SQL effect
    /// or claim; abandoned work is recovered by leases and durable redelivery.
    pub async fn shutdown(&self) -> Result<()> {
        self.consumer_stream.lock().await.take();
        tokio::time::timeout(IO_TIMEOUT, self.client.drain())
            .await
            .map_err(|_| EventError::Deadline)?
            .map_err(|_| EventError::Broker("drain"))
    }
}

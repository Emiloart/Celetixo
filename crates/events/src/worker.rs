//! Operator-configured service loop. Configuration is trusted private input,
//! never a workflow payload. Signals finish the current bounded batch, then drain.
use crate::{BatchReport, BrokerConfiguration, EventError, NatsDelivery};
use celetixo_engine::ids::{OrganizationId, RepositoryId, Scope, WorkspaceId};
use celetixo_postgres::{
    UnixConnection,
    delivery::{DeliveryStatus, EventDelivery, ServiceKind},
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfiguration {
    contract_version: u32,
    organization_id: String,
    workspace_id: String,
    repository_id: String,
    service_id: String,
    service_kind: WorkerKind,
    database: DatabaseConfiguration,
    broker: BrokerFiles,
    poll_interval_ms: u64,
    reconcile_interval_ms: u64,
}
#[derive(Clone, Copy, Deserialize)]
enum WorkerKind {
    Publisher,
    Consumer,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseConfiguration {
    directory: PathBuf,
    database: String,
    user: String,
    password_file: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerFiles {
    servers: Vec<String>,
    ca_file: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    subject: String,
    stream: String,
    consumer: String,
    inbox_prefix: String,
}

#[cfg(unix)]
fn private_bytes(path: &Path) -> Result<Vec<u8>, EventError> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    if !path.is_absolute() || path.canonicalize().map_err(|_| EventError::Configuration)? != path {
        return Err(EventError::Configuration);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| EventError::Configuration)?;
    let info = file.metadata().map_err(|_| EventError::Configuration)?;
    if !info.is_file()
        || info.uid() != rustix::process::getuid().as_raw()
        || info.mode() & 0o777 != 0o600
        || info.nlink() != 1
        || info.len() > 65536
    {
        return Err(EventError::Configuration);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| EventError::Configuration)?;
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err(EventError::Configuration);
    }
    Ok(bytes)
}
#[cfg(not(unix))]
fn private_bytes(_: &Path) -> Result<Vec<u8>, EventError> {
    Err(EventError::Configuration)
}

pub fn load(path: &Path) -> Result<WorkerConfiguration, EventError> {
    let configuration: WorkerConfiguration =
        serde_json::from_slice(&private_bytes(path)?).map_err(|_| EventError::Configuration)?;
    if configuration.contract_version != 1
        || !(100..=60_000).contains(&configuration.poll_interval_ms)
        || !(1_000..=300_000).contains(&configuration.reconcile_interval_ms)
    {
        return Err(EventError::Configuration);
    }
    Ok(configuration)
}

fn metric(
    status: &str,
    kind: WorkerKind,
    report: BatchReport,
    backlog: Option<DeliveryStatus>,
    failures: u32,
) {
    // Fixed keys, bounded numeric counters, no credentials or broker diagnostics.
    println!(
        "{}",
        serde_json::json!({"component":"celetixo-events","status":status,
        "service_kind":match kind {WorkerKind::Publisher=>"Publisher",WorkerKind::Consumer=>"Consumer"},
        "published":report.published,"applied":report.applied,"replayed":report.replayed,
        "quarantined":report.quarantined,"deferred":report.deferred,"consecutive_failures":failures,
        "outbox_pending":backlog.map(|b|b.pending),"outbox_claimed":backlog.map(|b|b.claimed),
        "outbox_quarantined":backlog.map(|b|b.quarantined),"unframed":backlog.map(|b|b.unframed),
        "oldest_pending_age_ms":backlog.map(|b|b.oldest_pending_age_ms)})
    );
}

pub async fn run(
    configuration: WorkerConfiguration,
    cancel: Arc<AtomicBool>,
) -> Result<(), EventError> {
    let scope = Scope {
        organization: OrganizationId::new(configuration.organization_id)
            .map_err(|_| EventError::Configuration)?,
        workspace: WorkspaceId::new(configuration.workspace_id)
            .map_err(|_| EventError::Configuration)?,
        repository: RepositoryId::new(configuration.repository_id)
            .map_err(|_| EventError::Configuration)?,
    };
    let kind = configuration.service_kind;
    let service = configuration.service_id;
    let database = configuration.database;
    let selected = scope.clone();
    let store = Arc::new(
        tokio::task::spawn_blocking(move || {
            let password = private_bytes(&database.password_file)?;
            let password = std::str::from_utf8(&password)
                .map_err(|_| EventError::Configuration)?
                .trim();
            let connection = UnixConnection::new(
                database.directory,
                &database.database,
                &database.user,
                password.as_bytes(),
            )
            .map_err(celetixo_postgres::delivery::DeliveryError::from)?;
            EventDelivery::connect(
                connection,
                selected,
                match kind {
                    WorkerKind::Publisher => ServiceKind::Publisher,
                    WorkerKind::Consumer => ServiceKind::Consumer,
                },
                &service,
            )
            .map_err(EventError::from)
        })
        .await
        .map_err(|_| EventError::DatabaseTask)??,
    );
    let files = configuration.broker;
    // Validate every secret/certificate before async-nats resolves its inputs.
    let (ca_file, certificate, private_key) = tokio::task::spawn_blocking(move || {
        for path in [&files.ca_file, &files.certificate, &files.private_key] {
            private_bytes(path)?;
        }
        Ok::<_, EventError>((files.ca_file, files.certificate, files.private_key))
    })
    .await
    .map_err(|_| EventError::DatabaseTask)??;
    let delivery = NatsDelivery::connect(BrokerConfiguration {
        scope,
        servers: files.servers,
        ca_file,
        certificate,
        private_key,
        subject: files.subject,
        stream: files.stream,
        consumer: files.consumer,
        inbox_prefix: files.inbox_prefix,
    })
    .await?;
    let mut last_reconcile = Instant::now();
    let mut failures = 0;
    while !cancel.load(Ordering::Acquire) {
        let result = match kind {
            WorkerKind::Publisher => delivery.publish_once(store.clone()).await,
            WorkerKind::Consumer => delivery.consume_once(store.clone()).await,
        };
        match result {
            Ok(report) => {
                let database = store.clone();
                let backlog = tokio::task::spawn_blocking(move || database.status())
                    .await
                    .map_err(|_| EventError::DatabaseTask)??;
                if matches!(kind, WorkerKind::Consumer)
                    && last_reconcile.elapsed()
                        >= Duration::from_millis(configuration.reconcile_interval_ms)
                {
                    delivery.reconcile_once(store.clone()).await?;
                    last_reconcile = Instant::now();
                }
                failures = if report.deferred > 0 { failures + 1 } else { 0 };
                metric(
                    if failures == 0 {
                        "ready"
                    } else {
                        "dependency_unavailable"
                    },
                    kind,
                    report,
                    Some(backlog),
                    failures,
                );
            }
            Err(error) => {
                failures += 1;
                metric(
                    "dependency_unavailable",
                    kind,
                    BatchReport::default(),
                    None,
                    failures,
                );
                if failures >= 8 {
                    return Err(error);
                }
            }
        }
        if failures >= 8 {
            return Err(EventError::Broker("retry_budget_exhausted"));
        }
        let delay = if failures == 0 {
            Duration::from_millis(configuration.poll_interval_ms)
        } else {
            Duration::from_secs(1_u64 << failures.min(5))
        };
        // Check cancellation at least every 100ms while idle/backing off.
        let deadline = Instant::now() + delay;
        while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    delivery.shutdown().await?;
    metric("stopped", kind, BatchReport::default(), None, failures);
    Ok(())
}

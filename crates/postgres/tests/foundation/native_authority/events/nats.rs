//! Actual three-node broker checks. Requires the owned fixture, not a public or
//! production stream. Test messages are retained; no stream is deleted/purged.
use super::*;
use async_nats::jetstream::{
    self,
    consumer::{AckPolicy, PullConsumer},
    stream::{Config, DiscardPolicy, RetentionPolicy, StorageType},
};
use celetixo_events::{BrokerConfiguration, NatsDelivery};
#[path = "nats/faults.rs"]
mod faults;
#[path = "nats/processes.rs"]
mod processes;

#[derive(Deserialize)]
struct Identity {
    certificate: PathBuf,
    private_key: PathBuf,
}
#[derive(Deserialize)]
struct BrokerFixture {
    servers: Vec<String>,
    ca_file: PathBuf,
    identities: BTreeMap<String, Identity>,
}
fn fixture_config(identity: &str, scope: &Scope) -> BrokerConfiguration {
    let path = std::env::var_os("CELETIXO_NATS_TEST_CONFIG")
        .expect("reviewed NATS fixture configuration is required");
    let fixture: BrokerFixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let selected = &fixture.identities[identity];
    BrokerConfiguration {
        scope: scope.clone(),
        servers: fixture.servers,
        ca_file: fixture.ca_file,
        certificate: selected.certificate.clone(),
        private_key: selected.private_key.clone(),
        subject: "celetixo.fixture.lifecycle".into(),
        stream: "LIFECYCLE".into(),
        consumer: "PROJECTION".into(),
        inbox_prefix: format!(
            "_INBOX.{}",
            if identity == "foreign-publisher" {
                "foreign"
            } else {
                identity
            }
        ),
    }
}
async fn raw(identity: &str) -> async_nats::Client {
    let c = fixture_config(identity, &super::scope("raw-broker-client"));
    async_nats::ConnectOptions::new()
        .add_root_certificates(c.ca_file)
        .add_client_certificate(c.certificate, c.private_key)
        .require_tls(true)
        .tls_first()
        .connection_timeout(Duration::from_secs(3))
        .request_timeout(Some(Duration::from_secs(3)))
        .max_reconnects(8)
        .custom_inbox_prefix(c.inbox_prefix)
        .connect(c.servers)
        .await
        .unwrap()
}

#[test]
#[ignore = "requires authenticated PostgreSQL event logins and the owned three-node mutual-TLS NATS fixture"]
fn real_jetstream_puback_redelivery_poison_retention_reconciliation_and_scoped_roles() {
    let mut h = Harness::new("nats-lifecycle");
    h.ready("a", "x", 10, true);
    let original_request = h.request("a", "nats-commit");
    let receipt = h.commit(original_request.clone()).unwrap();
    let publisher = Arc::new(service(
        h.scope(),
        "publisher",
        ServiceKind::Publisher,
        "nats-publisher",
    ));
    let consumer = Arc::new(service(
        h.scope(),
        "consumer",
        ServiceKind::Consumer,
        "nats-projection",
    ));
    let before = h.backend.inspect_native(h.scope()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let operator = runtime.block_on(raw("operator"));
    let mut js = runtime.block_on(async { jetstream::new(operator.clone()) });
    js.set_timeout(Duration::from_secs(3));
    let stream = runtime
        .block_on(js.get_or_create_stream(Config {
            name: "LIFECYCLE".into(),
            subjects: vec!["celetixo.fixture.lifecycle".into()],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            num_replicas: 3,
            max_bytes: 8 * 1024 * 1024,
            max_messages: 10_000,
            max_message_size: 1_048_576,
            max_age: Duration::from_secs(3600),
            discard: DiscardPolicy::Old,
            duplicate_window: Duration::from_secs(2),
            max_consumers: 4,
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(stream.cached_info().config.num_replicas, 3);
    // Each test invocation has a new SQL scope. Reset only this owned fixture's
    // derived consumer cursor; retain all stream messages and SQL facts. Process
    // crashes and broker restarts below preserve this new durable cursor.
    match runtime.block_on(js.delete_consumer_from_stream("PROJECTION", "LIFECYCLE")) {
        Ok(_) => {}
        Err(error) => assert!(
            matches!(error.kind(),jetstream::stream::ConsumerErrorKind::JetStream(ref cause)
            if cause.error_code()==jetstream::ErrorCode::CONSUMER_NOT_FOUND)
        ),
    }
    let start_sequence = stream
        .cached_info()
        .state
        .last_sequence
        .checked_add(1)
        .unwrap();
    let _: PullConsumer = runtime
        .block_on(stream.get_or_create_consumer(
            "PROJECTION",
            jetstream::consumer::pull::Config {
                durable_name: Some("PROJECTION".into()),
                deliver_policy: jetstream::consumer::DeliverPolicy::ByStartSequence {
                    start_sequence,
                },
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(1),
                max_deliver: 8,
                max_ack_pending: 16,
                max_waiting: 4,
                max_batch: 8,
                max_bytes: 8 * 1_048_576,
                filter_subject: "celetixo.fixture.lifecycle".into(),
                num_replicas: 3,
                backoff: vec![Duration::from_secs(1), Duration::from_secs(2)],
                ..Default::default()
            },
        ))
        .unwrap();
    let publishing = runtime
        .block_on(NatsDelivery::connect(fixture_config(
            "publisher",
            h.scope(),
        )))
        .unwrap();
    let consuming = runtime
        .block_on(NatsDelivery::connect(fixture_config("consumer", h.scope())))
        .unwrap();
    // Crash/lost response after broker persistence, before marking SQL delivered.
    let before_send = processes::run(h.scope(), "publisher_before_send");
    std::thread::sleep(Duration::from_millis(150));
    let after_ack = processes::run(h.scope(), "publisher_after_ack");
    assert_eq!(before_send["event_id"], after_ack["event_id"]);
    let original = (
        after_ack["stream"].as_str().unwrap().to_string(),
        after_ack["sequence"].as_u64().unwrap(),
    );
    std::thread::sleep(Duration::from_millis(150));
    let replacement = publisher
        .claim(1, Duration::from_secs(30), 8)
        .unwrap()
        .remove(0);
    let dedup = runtime
        .block_on(publishing.publish_claim(&replacement))
        .unwrap();
    assert_eq!(original, dedup);
    publisher
        .confirm_published(&replacement, &dedup.0, dedup.1)
        .unwrap();
    for _ in 0..128 {
        if publisher.status().unwrap().pending == 0 {
            break;
        }
        let report = runtime
            .block_on(publishing.publish_once(publisher.clone()))
            .unwrap();
        assert_eq!(report.deferred, 0);
    }
    assert_eq!(publisher.status().unwrap().pending, 0);
    // Consumer loses its process after SQL effect and before broker ack. Fetch
    // manually, apply, drop the message, then use the production consumer path.
    processes::run(h.scope(), "consumer_before_effect");
    std::thread::sleep(Duration::from_millis(1200));
    let abandoned = processes::run(h.scope(), "consumer_after_effect");
    let abandoned_bytes: Vec<u8> =
        serde_json::from_value(abandoned["event_bytes"].clone()).unwrap();
    std::thread::sleep(Duration::from_millis(2200));
    let mut replayed = 0;
    for _ in 0..128 {
        let report = runtime
            .block_on(consuming.consume_once(consumer.clone()))
            .unwrap();
        replayed += report.replayed;
        if report.applied + report.replayed + report.quarantined == 0
            && consumer.missing_frames(128).unwrap().is_empty()
        {
            break;
        }
    }
    assert!(replayed > 0);
    assert!(consumer.missing_frames(128).unwrap().is_empty());
    // Duplicate after the finite broker deduplication window is still exactly
    // one SQL effect. Stable IDs never substitute for the durable inbox.
    std::thread::sleep(Duration::from_millis(2100));
    runtime.block_on(async {
        js.publish("celetixo.fixture.lifecycle", abandoned_bytes.clone().into())
            .await
            .unwrap()
            .await
            .unwrap()
    });
    assert!(
        runtime
            .block_on(consuming.consume_once(consumer.clone()))
            .unwrap()
            .replayed
            > 0
    );
    runtime.block_on(async {
        js.publish("celetixo.fixture.lifecycle", b"poison".to_vec().into())
            .await
            .unwrap()
            .await
            .unwrap()
    });
    assert_eq!(
        runtime
            .block_on(consuming.consume_once(consumer.clone()))
            .unwrap()
            .quarantined,
        1
    );
    // Force actual bounded-retention eviction, then rebuild a fresh SQL inbox.
    // Only the explicitly owned test stream is changed; its normal limit is
    // restored and all PostgreSQL facts/evidence remain retained.
    let mut retention = stream.cached_info().config.clone();
    retention.max_messages = 2;
    runtime
        .block_on(js.update_stream(retention.clone()))
        .unwrap();
    runtime.block_on(async {
        for _ in 0..3 {
            js.publish(
                "celetixo.fixture.lifecycle",
                b"retention-fixture".to_vec().into(),
            )
            .await
            .unwrap()
            .await
            .unwrap();
        }
    });
    let retained = runtime.block_on(js.get_stream("LIFECYCLE")).unwrap();
    assert_eq!(retained.cached_info().state.messages, 2);
    assert!(retained.cached_info().state.first_sequence > 1);
    retention.max_messages = 10_000;
    runtime.block_on(js.update_stream(retention)).unwrap();
    // Retention/rebuild is recovered from immutable SQL facts for a fresh inbox,
    // independently of broker cursor or message retention.
    let rebuilt = Arc::new(service(
        h.scope(),
        "consumer",
        ServiceKind::Consumer,
        "rebuilt-projection",
    ));
    for _ in 0..128 {
        if runtime
            .block_on(consuming.reconcile_once(rebuilt.clone()))
            .unwrap()
            .applied
            == 0
        {
            break;
        }
    }
    assert!(rebuilt.missing_frames(128).unwrap().is_empty());
    let foreign = runtime.block_on(raw("foreign-publisher"));
    runtime.block_on(async {
        let foreign_js = jetstream::new(foreign);
        assert!(match foreign_js
            .publish(
                "celetixo.fixture.lifecycle",
                b"cross-account".to_vec().into()
            )
            .await
        {
            Ok(ack) => ack.await.is_err(),
            Err(_) => true,
        });
        let consumer_js = jetstream::new(raw("consumer").await);
        assert!(match consumer_js
            .publish("celetixo.fixture.lifecycle", b"wrong-role".to_vec().into())
            .await
        {
            Ok(ack) => ack.await.is_err(),
            Err(_) => true,
        });
        let publisher_js = jetstream::new(raw("publisher").await);
        assert!(publisher_js.get_stream("LIFECYCLE").await.is_err());
        assert!(
            NatsDelivery::connect(fixture_config("unmapped", h.scope()))
                .await
                .is_err()
        );
    });
    assert_eq!(h.backend.inspect_native(h.scope()).unwrap(), before);
    let (publishing, consuming) =
        faults::exercise(&mut h, &runtime, publisher, consumer, publishing, consuming);
    assert_eq!(h.commit(original_request), Ok(receipt));
    runtime.block_on(publishing.shutdown()).unwrap();
    runtime.block_on(consuming.shutdown()).unwrap();
}

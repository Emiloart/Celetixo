//! Real process exits at broker/SQL acknowledgement boundaries. Child outcomes
//! are observations, never an authorization or Engine commit decision.
use super::*;
use futures_util::StreamExt;
use std::io::Write;

const CASE: &str = "native_authority::events::nats::processes::event_process_checkpoint";

pub(super) fn run(selected: &Scope, operation: &str) -> serde_json::Value {
    assert!(matches!(
        operation,
        "publisher_before_send"
            | "publisher_after_ack"
            | "consumer_before_effect"
            | "consumer_after_effect"
    ));
    let root = tempfile::Builder::new()
        .prefix("celetixo-event-process-")
        .tempdir()
        .unwrap()
        .keep();
    let request = root.join("request.json");
    let value = serde_json::json!({"scope":[selected.organization.as_str(),selected.workspace.as_str(),selected.repository.as_str()],"operation":operation});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&request)
        .unwrap();
    file.write_all(serde_json::to_string(&value).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
    let stdout = std::fs::File::create(root.join("child.log")).unwrap();
    let stderr = stdout.try_clone().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", CASE, "--nocapture"])
        .env("CELETIXO_EVENT_PROCESS_REQUEST", &request)
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("event child exceeded deadline; retained {}", root.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        status.code(),
        Some(87),
        "process checkpoint failed; retained {}",
        root.display()
    );
    serde_json::from_slice(&std::fs::read(root.join("result.json")).unwrap()).unwrap()
}

#[test]
#[ignore = "child helper; requires an exact private parent request"]
fn event_process_checkpoint() {
    let Some(path) = std::env::var_os("CELETIXO_EVENT_PROCESS_REQUEST") else {
        return;
    };
    let path = PathBuf::from(path);
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let selectors: Vec<String> = serde_json::from_value(request["scope"].clone()).unwrap();
    assert_eq!(selectors.len(), 3);
    let selected = Scope {
        organization: celetixo_engine::ids::OrganizationId::new(&selectors[0]).unwrap(),
        workspace: celetixo_engine::ids::WorkspaceId::new(&selectors[1]).unwrap(),
        repository: celetixo_engine::ids::RepositoryId::new(&selectors[2]).unwrap(),
    };
    let operation = request["operation"].as_str().unwrap();
    let publisher = operation.starts_with("publisher_");
    let store = EventDelivery::connect(
        connection(if publisher { "publisher" } else { "consumer" }, None),
        selected.clone(),
        if publisher {
            ServiceKind::Publisher
        } else {
            ServiceKind::Consumer
        },
        if publisher {
            "nats-publisher"
        } else {
            "nats-projection"
        },
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = if publisher {
        let claim = store
            .claim(1, Duration::from_millis(100), 8)
            .unwrap()
            .remove(0);
        if operation == "publisher_before_send" {
            serde_json::json!({"event_id":claim.event().event().event_id})
        } else {
            assert_eq!(operation, "publisher_after_ack");
            let delivery = runtime
                .block_on(NatsDelivery::connect(fixture_config(
                    "publisher",
                    &selected,
                )))
                .unwrap();
            let ack = runtime.block_on(delivery.publish_claim(&claim)).unwrap();
            serde_json::json!({"stream":ack.0,"sequence":ack.1,"event_id":claim.event().event().event_id})
        }
    } else {
        let operator = runtime.block_on(raw("operator"));
        let js = runtime.block_on(async { jetstream::new(operator) });
        let consumer: PullConsumer = runtime
            .block_on(js.get_consumer_from_stream("PROJECTION", "LIFECYCLE"))
            .unwrap();
        let message = runtime.block_on(async {
            let mut batch = consumer.fetch().max_messages(1).messages().await.unwrap();
            batch.next().await.unwrap().unwrap()
        });
        if operation == "consumer_after_effect" {
            store.apply(&message.payload).unwrap();
        } else {
            assert_eq!(operation, "consumer_before_effect");
        }
        serde_json::json!({"event_bytes":message.payload.to_vec()})
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.parent().unwrap().join("result.json"))
        .unwrap();
    file.write_all(serde_json::to_string(&result).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
    // No stack unwinding, client drain, claim release, or message ack occurs.
    std::process::exit(87);
}

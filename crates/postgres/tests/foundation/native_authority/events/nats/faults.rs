use super::*;

struct StoppedNode {
    name: String,
    stopped: bool,
}
impl StoppedNode {
    fn stop(node: &str, crash: bool) -> Self {
        assert!(matches!(node, "n1" | "n2" | "n3"));
        let path = PathBuf::from(std::env::var_os("CELETIXO_NATS_TEST_CONFIG").unwrap());
        let root = path.parent().unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        let name = format!("{}-{node}", manifest["network"].as_str().unwrap());
        let output = Command::new("docker")
            .args(["inspect", &name])
            .output()
            .unwrap();
        assert!(output.status.success());
        let inspected: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            inspected[0]["Config"]["Labels"]["dev.celetixo.nats-fixture-root"].as_str(),
            root.to_str()
        );
        assert_eq!(inspected[0]["Image"], manifest["image_id"]);
        let output = if crash {
            Command::new("docker")
                .args(["kill", "--signal=KILL", &name])
                .output()
        } else {
            Command::new("docker")
                .args(["stop", "--time=3", &name])
                .output()
        }
        .unwrap();
        assert!(output.status.success());
        Self {
            name,
            stopped: true,
        }
    }
    fn restart(&mut self) {
        let output = Command::new("docker")
            .args(["start", &self.name])
            .output()
            .unwrap();
        assert!(output.status.success());
        self.stopped = false;
    }
}
impl Drop for StoppedNode {
    fn drop(&mut self) {
        if self.stopped {
            // Best-effort cleanup cannot turn a failed assertion into a pass.
            let _ = Command::new("docker").args(["start", &self.name]).output();
        }
    }
}

fn drain(
    runtime: &tokio::runtime::Runtime,
    delivery: &NatsDelivery,
    publisher: &Arc<EventDelivery>,
) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while publisher.status().unwrap().pending > 0 {
        assert!(
            Instant::now() < deadline,
            "outbox recovery exceeded finite test deadline"
        );
        runtime
            .block_on(delivery.publish_once(publisher.clone()))
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(publisher.status().unwrap().quarantined, 0);
}

pub(super) fn exercise(
    h: &mut Harness,
    runtime: &tokio::runtime::Runtime,
    publisher: Arc<EventDelivery>,
    consumer: Arc<EventDelivery>,
    publishing: NatsDelivery,
    consuming: NatsDelivery,
) -> (NatsDelivery, NatsDelivery) {
    let operator = runtime.block_on(raw("operator"));
    let js = runtime.block_on(async { jetstream::new(operator) });
    let info = runtime.block_on(js.get_stream("LIFECYCLE")).unwrap();
    let leader = info
        .cached_info()
        .cluster
        .as_ref()
        .unwrap()
        .leader
        .as_ref()
        .unwrap()
        .clone();
    let previous_last = info.cached_info().state.last_sequence;
    let mut killed = StoppedNode::stop(&leader, true);
    h.engine
        .propose(
            &h.worker,
            h.scope(),
            id("broker-leader-down"),
            h.base.id(),
            h.intent("y"),
        )
        .unwrap();
    drain(runtime, &publishing, &publisher);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(info) = runtime.block_on(js.get_stream("LIFECYCLE"))
            && info
                .cached_info()
                .cluster
                .as_ref()
                .unwrap()
                .leader
                .as_deref()
                != Some(&leader)
            && info.cached_info().state.last_sequence > previous_last
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "replicated stream did not elect another leader"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    killed.restart();
    // The next prepared native commit executes with two brokers stopped. A lone
    // replica must not acknowledge publication without a stream quorum.
    let current = h.engine.current(&h.worker, h.scope()).unwrap();
    h.ready_at("broker-outage-commit", &current, "y", 20, h.intent("y"));
    let request = h.request("broker-outage-commit", "broker-outage-key");
    let mut second = StoppedNode::stop("n2", false);
    let mut third = StoppedNode::stop("n3", false);
    let committed = h.commit(request.clone()).unwrap();
    assert_eq!(committed.revision.0, 2);
    let truth = h.backend.inspect_native(h.scope()).unwrap();
    let report = runtime
        .block_on(publishing.publish_once(publisher.clone()))
        .unwrap();
    assert_eq!(report.published, 0);
    assert!(report.deferred > 0);
    assert!(publisher.status().unwrap().pending > 0);
    let mut first = StoppedNode::stop("n1", false);
    h.reopen();
    assert_eq!(h.backend.inspect_native(h.scope()).unwrap(), truth);
    assert_eq!(h.commit(request), Ok(committed));
    let recovered = h.backend.inspect_native(h.scope()).unwrap();
    // Replaying an authenticated Engine update records a fresh publication and
    // observed clock. The original semantic result and every domain row remain
    // exact; broker outages cannot create another revision or terminal outcome.
    assert_eq!(recovered.state_generation, truth.state_generation + 1);
    let mut replay_expected = truth;
    replay_expected.state_generation = recovered.state_generation;
    assert!(recovered.records.clock_ms >= replay_expected.records.clock_ms);
    replay_expected.records.clock_ms = recovered.records.clock_ms;
    assert!(
        recovered == replay_expected,
        "commit recovery changed domain truth"
    );
    let truth = recovered;
    first.restart();
    second.restart();
    third.restart();
    // Exhausted transport reconnects require fresh workload connections, while
    // the durable identities and outstanding claims/inbox remain unchanged.
    drop(publishing);
    drop(consuming);
    let publishing = runtime
        .block_on(NatsDelivery::connect(fixture_config(
            "publisher",
            h.scope(),
        )))
        .unwrap();
    let consuming = runtime
        .block_on(NatsDelivery::connect(fixture_config("consumer", h.scope())))
        .unwrap();
    drain(runtime, &publishing, &publisher);
    for _ in 0..128 {
        let report = runtime
            .block_on(consuming.consume_once(consumer.clone()))
            .unwrap();
        if report.applied + report.replayed + report.quarantined == 0
            && consumer.missing_frames(128).unwrap().is_empty()
        {
            break;
        }
    }
    assert!(consumer.missing_frames(128).unwrap().is_empty());
    assert_eq!(h.backend.inspect_native(h.scope()).unwrap(), truth);
    let operator = runtime.block_on(raw("operator"));
    let js = runtime.block_on(async { jetstream::new(operator) });
    let reopened = runtime.block_on(js.get_stream("LIFECYCLE")).unwrap();
    assert_eq!(reopened.cached_info().config.num_replicas, 3);
    assert!(reopened.cached_info().state.last_sequence > previous_last);
    (publishing, consuming)
}

use super::*;
use celetixo_postgres::delivery::{DeliveryError, EventDelivery, QuarantineReason, ServiceKind};
use celetixo_protocol::celetixo::platform::v1::LifecycleEvent;
use prost::Message;
#[path = "events/clock.rs"]
pub(crate) mod clock;
#[path = "events/nats.rs"]
mod nats;
#[path = "events/poison.rs"]
mod poison;
#[path = "events/upgrade.rs"]
mod upgrade;

pub(crate) fn service(
    selected: &Scope,
    role: &str,
    kind: ServiceKind,
    service_id: &str,
) -> EventDelivery {
    let mut sql = owner();
    let config = fixture();
    let name = &config.roles[role].user;
    assert!(
        name.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    );
    sql.execute("INSERT INTO celetixo.db_role_scopes (database_role,database_role_oid,organization_id,workspace_id,repository_id) SELECT rolname,oid,$2::text,$3::text,$4::text FROM pg_roles WHERE rolname=$1 ON CONFLICT DO NOTHING", &[name,&selected.organization.as_str(),&selected.workspace.as_str(),&selected.repository.as_str()]).unwrap();
    let kind_name = match kind {
        ServiceKind::Publisher => "Publisher",
        ServiceKind::Consumer => "Consumer",
    };
    sql.execute("INSERT INTO celetixo.event_service_roles (database_role,database_role_oid,organization_id,workspace_id,repository_id,service_kind,service_id) SELECT rolname,oid,$2::text,$3::text,$4::text,$5,$6::text FROM pg_roles WHERE rolname=$1 ON CONFLICT DO NOTHING", &[name,&selected.organization.as_str(),&selected.workspace.as_str(),&selected.repository.as_str(),&kind_name,&service_id]).unwrap();
    sql.batch_execute(&format!("GRANT USAGE ON SCHEMA celetixo TO {name}; GRANT SELECT ON celetixo.schema_migrations,celetixo.repositories,celetixo.lifecycle_frames,celetixo.domain_payloads,celetixo.outbox,celetixo.outbox_delivery TO {name}; GRANT EXECUTE ON FUNCTION celetixo.has_repository_scope(text,text,text),celetixo.has_event_service_scope(text,text,text,text,text),celetixo.event_consumer_identity(text,text,text,text) TO {name};")).unwrap();
    match kind {
        ServiceKind::Publisher=>sql.batch_execute(&format!("GRANT UPDATE ON celetixo.outbox_delivery TO {name};")).unwrap(),
        ServiceKind::Consumer=>sql.batch_execute(&format!("GRANT SELECT ON celetixo.repository_heads,celetixo.transactions,celetixo.inbox,celetixo.event_projections,celetixo.event_quarantine TO {name}; GRANT INSERT ON celetixo.inbox,celetixo.event_projections,celetixo.event_quarantine TO {name}; GRANT UPDATE ON celetixo.event_projections TO {name};")).unwrap(),
    }
    EventDelivery::connect(connection(role, None), selected.clone(), kind, service_id).unwrap()
}

#[test]
#[ignore = "requires independent authenticated PostgreSQL publisher/consumer logins"]
fn lifecycle_outbox_fences_replays_reorders_quarantines_and_preserves_engine_truth() {
    let mut h = Harness::new("event-delivery");
    h.ready("a", "x", 10, true);
    let request = h.request("a", "event-commit");
    let receipt = h.commit(request.clone()).unwrap();
    let publisher = service(
        h.scope(),
        "publisher",
        ServiceKind::Publisher,
        "publisher-a",
    );
    let consumer = service(h.scope(), "consumer", ServiceKind::Consumer, "projection-a");
    let before = h.backend.inspect_native(h.scope()).unwrap();
    let status = publisher.status().unwrap();
    assert!(status.pending > before.state_generation); // state/commit facts accompany publication markers
    assert_eq!(status.unframed, 0);
    let mut claims = publisher.claim(128, Duration::from_secs(30), 8).unwrap();
    assert_eq!(claims.len() as u64, status.pending);
    assert!(
        publisher
            .claim(128, Duration::from_secs(30), 8)
            .unwrap()
            .is_empty()
    );
    // Deliberately deliver the latest generation first for every aggregate.
    claims.sort_by_key(|claim| std::cmp::Reverse(claim.event().event().aggregate_sequence));
    for (index, claim) in claims.iter().enumerate() {
        let result = consumer.apply(claim.bytes()).unwrap();
        assert!(!result.replayed);
        assert_eq!(result.current_revision, 1);
        let replay = consumer.apply(claim.bytes()).unwrap();
        assert!(replay.replayed);
        assert!(!replay.projection_advanced);
        publisher
            .confirm_published(claim, "LIFECYCLE", index as u64 + 1)
            .unwrap();
    }
    assert_eq!(publisher.status().unwrap().pending, 0);
    // A consumer restart still returns the original durable inbox identity.
    let reopened = EventDelivery::connect(
        connection("consumer", None),
        h.scope().clone(),
        ServiceKind::Consumer,
        "projection-a",
    )
    .unwrap();
    assert!(reopened.apply(claims[0].bytes()).unwrap().replayed);
    let mut forged = LifecycleEvent::decode(claims[0].bytes()).unwrap();
    forged.trace_id.push_str("-substituted");
    assert!(matches!(
        consumer.apply(&forged.encode_to_vec()),
        Err(DeliveryError::Substitution)
    ));
    forged.scope.as_mut().unwrap().workspace_id = "foreign".into();
    assert!(matches!(
        consumer.apply(&forged.encode_to_vec()),
        Err(DeliveryError::Scope)
    ));
    assert!(matches!(
        consumer.apply(b"poison"),
        Err(DeliveryError::Schema)
    ));
    let quarantine = consumer
        .quarantine(b"poison", QuarantineReason::Schema)
        .unwrap();
    assert_eq!(
        consumer
            .quarantine(b"poison", QuarantineReason::Schema)
            .unwrap(),
        quarantine
    );
    assert_eq!(h.backend.inspect_native(h.scope()).unwrap(), before);
    assert_eq!(h.commit(request), Ok(receipt));
    // A new Engine publication proceeds while the publisher is unavailable.
    h.engine
        .propose(
            &h.worker,
            h.scope(),
            id("later"),
            h.base.id(),
            h.intent("y"),
        )
        .unwrap();
    assert!(publisher.status().unwrap().pending > 0);
    let old = publisher
        .claim(1, Duration::from_millis(100), 8)
        .unwrap()
        .remove(0);
    std::thread::sleep(Duration::from_millis(150));
    let replacement = publisher
        .claim(1, Duration::from_secs(30), 8)
        .unwrap()
        .remove(0);
    assert_eq!(
        old.event().event().event_id,
        replacement.event().event().event_id
    );
    assert!(replacement.generation() > old.generation());
    assert!(matches!(
        publisher.confirm_published(&old, "LIFECYCLE", 1),
        Err(DeliveryError::LeaseLost)
    ));
    publisher
        .confirm_published(&replacement, "LIFECYCLE", 999)
        .unwrap();
    assert!(h.backend.inspect_native(h.scope()).is_ok());
}

#[test]
#[ignore = "requires independent authenticated PostgreSQL publisher/consumer logins"]
fn event_database_roles_cannot_assume_engine_or_other_consumer_authority() {
    let h = Harness::new("event-role-boundary");
    let publisher = service(
        h.scope(),
        "publisher",
        ServiceKind::Publisher,
        "publisher-b",
    );
    let consumer = service(h.scope(), "consumer", ServiceKind::Consumer, "projection-b");
    assert!(
        EventDelivery::connect(
            connection("runtime", None),
            h.scope().clone(),
            ServiceKind::Publisher,
            "publisher-b"
        )
        .is_err()
    );
    assert!(matches!(
        EventDelivery::connect(
            connection("consumer", None),
            h.scope().clone(),
            ServiceKind::Consumer,
            "projection-foreign"
        ),
        Err(DeliveryError::Unauthorized)
    ));
    assert!(matches!(
        EventDelivery::connect(
            connection("publisher", None),
            scope("event-foreign"),
            ServiceKind::Publisher,
            "publisher-b"
        ),
        Err(DeliveryError::Unauthorized)
    ));
    assert!(matches!(
        consumer.claim(1, Duration::from_secs(1), 8),
        Err(DeliveryError::Unauthorized)
    ));
    assert!(matches!(
        publisher.quarantine(b"poison", QuarantineReason::Schema),
        Err(DeliveryError::Unauthorized)
    ));
    for role in ["publisher", "consumer"] {
        let mut sql = connection(role, None).connect().unwrap();
        assert!(
            sql.batch_execute("UPDATE celetixo.repository_heads SET revision=revision")
                .is_err()
        );
        assert!(
            sql.batch_execute("UPDATE celetixo.transactions SET state=state")
                .is_err()
        );
        assert!(
            sql.batch_execute("DELETE FROM celetixo.event_service_roles")
                .is_err()
        );
    }
    let claim = publisher
        .claim(1, Duration::from_secs(30), 8)
        .unwrap()
        .remove(0);
    consumer.apply(claim.bytes()).unwrap();
    let mut sql = connection("consumer", None).connect().unwrap();
    let s = h.scope();
    assert!(sql.execute("INSERT INTO celetixo.inbox (organization_id,workspace_id,repository_id,consumer_id,event_id,event_payload_digest,result_payload_digest) SELECT organization_id,workspace_id,repository_id,'projection-foreign',event_id,event_payload_digest,result_payload_digest FROM celetixo.inbox WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&s.organization.as_str(),&s.workspace.as_str(),&s.repository.as_str()]).is_err());
    assert!(h.backend.inspect_native(h.scope()).is_ok());
}

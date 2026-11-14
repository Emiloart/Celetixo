use super::*;

#[test]
#[ignore = "requires independent authenticated PostgreSQL event logins"]
fn unsupported_outbox_frames_and_expired_final_claims_are_durably_quarantined() {
    let h = Harness::new("event-poison");
    let publisher = service(
        h.scope(),
        "publisher",
        ServiceKind::Publisher,
        "poison-publisher",
    );
    let before = h.backend.inspect_native(h.scope()).unwrap();
    let s = h.scope();
    let a = [
        s.organization.as_str(),
        s.workspace.as_str(),
        s.repository.as_str(),
    ];
    let mut sql = owner();
    let row=sql.query_one("SELECT p.payload_bytes FROM celetixo.lifecycle_frames AS f JOIN celetixo.domain_payloads AS p ON p.organization_id=f.organization_id AND p.workspace_id=f.workspace_id AND p.repository_id=f.repository_id AND p.payload_digest=f.frame_digest WHERE f.organization_id=$1::text AND f.workspace_id=$2::text AND f.repository_id=$3::text ORDER BY f.publication_generation DESC LIMIT 1", &[&a[0],&a[1],&a[2]]).unwrap();
    let mut future = LifecycleEvent::decode(row.get::<_, Vec<u8>>(0).as_slice()).unwrap();
    future.contract_version = 2;
    future.aggregate_sequence = before.state_generation + 1000;
    let event = Digest::of("outbox-schema-rejection-test/v1", a[0].as_bytes());
    future.event_id = event.as_bytes().to_vec();
    let bytes = future.encode_to_vec();
    let frame = Digest::of("celetixo/lifecycle-frame/v1", &bytes);
    let mut tx = sql.transaction().unwrap();
    tx.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,'celetixo/lifecycle-frame/v1',1,$5)", &[&a[0],&a[1],&a[2],&&frame.as_bytes()[..],&bytes]).unwrap();
    tx.execute("INSERT INTO celetixo.outbox (organization_id,workspace_id,repository_id,event_id,aggregate_kind,aggregate_id,aggregate_sequence,event_type,schema_version,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,'Authority',$3::text,$5::text::numeric,'LifecycleEvent',2,$6::bytea)", &[&a[0],&a[1],&a[2],&&event.as_bytes()[..],&future.aggregate_sequence.to_string(),&future.authority_payload_digest]).unwrap();
    tx.execute("INSERT INTO celetixo.outbox_delivery (organization_id,workspace_id,repository_id,event_id) VALUES ($1::text,$2::text,$3::text,$4::bytea)", &[&a[0],&a[1],&a[2],&&event.as_bytes()[..]]).unwrap();
    tx.execute("INSERT INTO celetixo.lifecycle_frames (organization_id,workspace_id,repository_id,event_id,frame_digest,publication_generation) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::text::numeric)", &[&a[0],&a[1],&a[2],&&event.as_bytes()[..],&&frame.as_bytes()[..],&before.state_generation.to_string()]).unwrap();
    tx.commit().unwrap();
    let claims = publisher.claim(128, Duration::from_secs(30), 8).unwrap();
    assert_eq!(claims.len() as u64, before.state_generation);
    assert!(
        claims
            .iter()
            .all(|claim| claim.event().event().event_id != event.as_bytes())
    );
    assert_eq!(publisher.status().unwrap().quarantined, 1);
    for (index, claim) in claims.iter().enumerate() {
        // Synthetic receipt tests the SQL state machine only. The real broker
        // parent separately requires actual replicated JetStream PubAcks.
        publisher
            .confirm_published(claim, "SQL_FIXTURE", index as u64 + 1)
            .unwrap();
    }
    assert!(
        publisher
            .claim(128, Duration::from_secs(30), 8)
            .unwrap()
            .is_empty()
    );
    let retained:Vec<u8>=sql.query_one("SELECT payload_bytes FROM celetixo.domain_payloads WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND payload_digest=$4::bytea", &[&a[0],&a[1],&a[2],&&frame.as_bytes()[..]]).unwrap().get(0);
    assert_eq!(retained, bytes);
    // An unknown future notification is not silently admitted as current truth.
    assert_eq!(
        h.backend.inspect_native(s),
        Err(AuthorityFailure::Integrity)
    );

    let healthy = Harness::new("event-exhaustion");
    let original = healthy.backend.inspect_native(healthy.scope()).unwrap();
    let publisher = service(
        healthy.scope(),
        "publisher",
        ServiceKind::Publisher,
        "exhaustion-publisher",
    );
    let abandoned = publisher
        .claim(1, Duration::from_millis(100), 1)
        .unwrap()
        .remove(0);
    std::thread::sleep(Duration::from_millis(150));
    let replacements = publisher.claim(128, Duration::from_secs(30), 1).unwrap();
    assert!(
        replacements
            .iter()
            .all(|claim| claim.event().event().event_id != abandoned.event().event().event_id)
    );
    assert_eq!(publisher.status().unwrap().quarantined, 1);
    assert!(matches!(
        publisher.confirm_published(&abandoned, "SQL_FIXTURE", 1),
        Err(DeliveryError::LeaseLost)
    ));
    assert_eq!(
        healthy.backend.inspect_native(healthy.scope()).unwrap(),
        original
    );
}

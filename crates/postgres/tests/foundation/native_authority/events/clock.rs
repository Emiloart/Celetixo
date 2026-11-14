//! Owner-injected coherent future-clock fixture. Every new authority publication
//! now includes its required immutable lifecycle fact; old rows stay untouched.
use super::*;
use celetixo_protocol::celetixo::platform::v1::{
    AggregateKind, LifecycleEventKind, RepositoryAuthorityChanged, ScopeSelector,
};

pub(crate) fn append_future_clock(owner: &mut Client, selected: &Scope, snapshot: &Snapshot) {
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let mut tx = owner.transaction().unwrap();
    let generation = tx.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get::<_,String>(0).parse::<u64>().unwrap().checked_add(1).unwrap();
    tx.execute("INSERT INTO celetixo.authority_publications (organization_id,workspace_id,repository_id,state_generation,observed_clock_ms,authenticated_database_role,authenticated_database_role_oid,actor_kind,snapshot_digest) SELECT organization_id,workspace_id,repository_id,$4::text::numeric,'18446744073709551615',session_user::name,(SELECT oid FROM pg_roles WHERE rolname=session_user),'EmbeddingAdministrator',snapshot_digest FROM celetixo.authority_publications WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text ORDER BY state_generation DESC LIMIT 1", &[&a[0],&a[1],&a[2],&generation.to_string()]).unwrap();
    tx.execute("UPDATE celetixo.repositories SET state_generation=$4::text::numeric WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2],&generation.to_string()]).unwrap();
    let mut metadata = Vec::new();
    for field in a {
        metadata.extend_from_slice(&(field.len() as u64).to_be_bytes());
        metadata.extend_from_slice(field.as_bytes());
    }
    metadata.extend_from_slice(&generation.to_be_bytes());
    metadata.extend_from_slice(&u64::MAX.to_be_bytes());
    metadata.extend_from_slice(snapshot.id().0.as_bytes());
    let metadata_format = "celetixo/event-authority-publication/v1";
    let metadata_digest = Digest::of(metadata_format, &metadata);
    let fact = RepositoryAuthorityChanged {
        revision: snapshot.revision().0,
        snapshot_digest: snapshot.id().0.as_bytes().to_vec(),
        metadata_digest: metadata_digest.as_bytes().to_vec(),
    };
    let fact_format = "celetixo/lifecycle-fact/v1";
    let fact_bytes = fact.encode_to_vec();
    let fact_digest = Digest::of(fact_format, &fact_bytes);
    let mut event = LifecycleEvent {
        contract_version: 1,
        scope: Some(ScopeSelector {
            organization_id: a[0].into(),
            workspace_id: a[1].into(),
            repository_id: a[2].into(),
        }),
        aggregate_kind: AggregateKind::Authority as i32,
        aggregate_id: a[2].into(),
        aggregate_sequence: generation,
        event_kind: LifecycleEventKind::RepositoryAuthorityChanged as i32,
        authority_payload_digest: fact_digest.as_bytes().to_vec(),
        trace_id: format!("authority:{generation}"),
        authority: Some(fact),
        ..Default::default()
    };
    event.event_id = Digest::of("celetixo/lifecycle-event-id/v1", &event.encode_to_vec())
        .as_bytes()
        .to_vec();
    let frame_format = "celetixo/lifecycle-frame/v1";
    let frame = event.encode_to_vec();
    let frame_digest = Digest::of(frame_format, &frame);
    for (format, digest, bytes) in [
        (metadata_format, metadata_digest, metadata),
        (fact_format, fact_digest, fact_bytes),
        (frame_format, frame_digest, frame),
    ] {
        tx.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,1,$6)", &[&a[0],&a[1],&a[2],&&digest.as_bytes()[..],&format,&bytes]).unwrap();
    }
    tx.execute("INSERT INTO celetixo.outbox (organization_id,workspace_id,repository_id,event_id,aggregate_kind,aggregate_id,aggregate_sequence,event_type,schema_version,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,'Authority',$3::text,$5::text::numeric,'LifecycleEvent',1,$6::bytea)", &[&a[0],&a[1],&a[2],&event.event_id,&generation.to_string(),&&fact_digest.as_bytes()[..]]).unwrap();
    tx.execute("INSERT INTO celetixo.outbox_delivery (organization_id,workspace_id,repository_id,event_id) VALUES ($1::text,$2::text,$3::text,$4::bytea)", &[&a[0],&a[1],&a[2],&event.event_id]).unwrap();
    tx.execute("INSERT INTO celetixo.lifecycle_frames (organization_id,workspace_id,repository_id,event_id,frame_digest,publication_generation) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::text::numeric)", &[&a[0],&a[1],&a[2],&event.event_id,&&frame_digest.as_bytes()[..],&generation.to_string()]).unwrap();
    tx.commit().unwrap();
}

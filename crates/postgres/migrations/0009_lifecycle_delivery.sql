-- Additive event delivery metadata. Engine truth and existing commit event IDs,
-- payloads and aggregate sequences retain their published meanings.
ALTER TABLE celetixo.outbox DROP CONSTRAINT outbox_aggregate_kind_check;
ALTER TABLE celetixo.outbox ADD CONSTRAINT outbox_aggregate_kind_check
    CHECK (aggregate_kind IN ('Repository','Transaction','SourceAttempt','Mutation','Authority'));

-- Explicit upgrade origin: old committed events can acquire wire frames during
-- checked reconciliation without inventing pre-upgrade lifecycle notifications.
CREATE TABLE celetixo.event_stream_origins (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    state_generation celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id) REFERENCES celetixo.repositories ON DELETE RESTRICT
);
INSERT INTO celetixo.event_stream_origins
    SELECT organization_id,workspace_id,repository_id,state_generation FROM celetixo.repositories;

CREATE TABLE celetixo.lifecycle_frames (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    event_id celetixo.digest32 NOT NULL,
    frame_digest celetixo.digest32 NOT NULL,
    publication_generation celetixo.uint64 NOT NULL CHECK (publication_generation>0),
    PRIMARY KEY (organization_id,workspace_id,repository_id,event_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,event_id) REFERENCES celetixo.outbox ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,frame_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,publication_generation) REFERENCES celetixo.authority_publications ON DELETE RESTRICT
);
ALTER TABLE celetixo.outbox_delivery
    ADD COLUMN claim_owner celetixo.identity,
    ADD COLUMN claim_expires_at TIMESTAMPTZ,
    ADD COLUMN quarantined_at TIMESTAMPTZ,
    ADD COLUMN broker_stream celetixo.identity,
    ADD COLUMN broker_sequence celetixo.uint64 CHECK (broker_sequence>0),
    ADD COLUMN last_error TEXT CHECK (octet_length(last_error)<=512),
    ADD CONSTRAINT outbox_claim_pair CHECK ((claim_owner IS NULL)=(claim_expires_at IS NULL));

CREATE TABLE celetixo.event_projections (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    consumer_id celetixo.identity NOT NULL,
    aggregate_kind TEXT NOT NULL CHECK (aggregate_kind IN ('Repository','Transaction','SourceAttempt','Mutation','Authority')),
    aggregate_id celetixo.identity NOT NULL,
    aggregate_sequence celetixo.uint64 NOT NULL,
    event_id celetixo.digest32 NOT NULL,
    frame_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,consumer_id,aggregate_kind,aggregate_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,event_id) REFERENCES celetixo.lifecycle_frames ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,frame_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.event_quarantine (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    consumer_id celetixo.identity NOT NULL,
    message_digest celetixo.digest32 NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('Schema','Scope','Authority','Substitution','RedeliveryLimit')),
    message_bytes BYTEA NOT NULL CHECK (octet_length(message_bytes)<=1048576),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id,workspace_id,repository_id,consumer_id,message_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id) REFERENCES celetixo.repositories ON DELETE RESTRICT
);

-- Names plus OIDs prevent a dropped/recreated login inheriting service access.
-- This table is administrator owned and never writable by event workloads.
CREATE TABLE celetixo.event_service_roles (
    database_role NAME NOT NULL,
    database_role_oid OID NOT NULL,
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    service_kind TEXT NOT NULL CHECK (service_kind IN ('Publisher','Consumer')),
    service_id celetixo.identity NOT NULL,
    PRIMARY KEY (database_role_oid,organization_id,workspace_id,repository_id,service_kind,service_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id) REFERENCES celetixo.repositories ON DELETE RESTRICT
);
-- Only the schema owner's authenticated administration and its narrow SECURITY
-- DEFINER predicates can inspect/change bindings. Workloads never own these
-- objects or inherit owner membership. Scope predicates still check session_user.
ALTER TABLE celetixo.event_service_roles ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.event_service_roles FORCE ROW LEVEL SECURITY;
CREATE POLICY mapping_owner ON celetixo.event_service_roles
    USING (current_user=pg_catalog.pg_get_userbyid((SELECT relowner FROM pg_catalog.pg_class WHERE oid='celetixo.event_service_roles'::regclass)))
    WITH CHECK (current_user=pg_catalog.pg_get_userbyid((SELECT relowner FROM pg_catalog.pg_class WHERE oid='celetixo.event_service_roles'::regclass)));
CREATE FUNCTION celetixo.has_event_service_scope(o TEXT,w TEXT,r TEXT,k TEXT,i TEXT)
RETURNS BOOLEAN LANGUAGE SQL STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT EXISTS (
        SELECT 1 FROM celetixo.event_service_roles AS binding
        JOIN pg_catalog.pg_roles AS login ON login.oid=binding.database_role_oid AND login.rolname=binding.database_role
        WHERE binding.database_role=session_user::name AND binding.organization_id=o
          AND binding.workspace_id=w AND binding.repository_id=r
          AND binding.service_kind=k AND binding.service_id=i
    )
$$;
REVOKE ALL ON FUNCTION celetixo.has_event_service_scope(TEXT,TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;
REVOKE ALL ON celetixo.event_service_roles FROM PUBLIC;

-- Workloads sharing a repository still cannot write another consumer's inbox,
-- projection or quarantine. Engine readers retain complete integrity visibility.
CREATE FUNCTION celetixo.event_consumer_identity(o TEXT,w TEXT,r TEXT,i TEXT)
RETURNS BOOLEAN LANGUAGE SQL STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM celetixo.event_service_roles AS b
        JOIN pg_catalog.pg_roles AS login ON login.oid=b.database_role_oid AND login.rolname=b.database_role
        WHERE b.database_role=session_user::name
    ) OR celetixo.has_event_service_scope(o,w,r,'Consumer',i)
$$;
REVOKE ALL ON FUNCTION celetixo.event_consumer_identity(TEXT,TEXT,TEXT,TEXT) FROM PUBLIC;
CREATE POLICY consumer_identity ON celetixo.inbox AS RESTRICTIVE
    USING (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id))
    WITH CHECK (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id));

DO $$
DECLARE target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY['event_stream_origins','lifecycle_frames','event_projections','event_quarantine'] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY',target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY',target);
        EXECUTE format('CREATE POLICY scope_isolation ON celetixo.%I USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id)) WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))',target);
        EXECUTE format('REVOKE ALL ON celetixo.%I FROM PUBLIC',target);
    END LOOP;
    FOREACH target IN ARRAY ARRAY['event_stream_origins','lifecycle_frames','event_quarantine'] LOOP
        EXECUTE format('CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.%I FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change()',target);
    END LOOP;
END;
$$;
CREATE POLICY consumer_identity ON celetixo.event_projections AS RESTRICTIVE
    USING (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id))
    WITH CHECK (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id));
CREATE POLICY consumer_identity ON celetixo.event_quarantine AS RESTRICTIVE
    USING (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id))
    WITH CHECK (celetixo.event_consumer_identity(organization_id,workspace_id,repository_id,consumer_id));

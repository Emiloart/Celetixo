-- Additive source lifecycle integration. Published migrations 0001-0003 remain
-- immutable. Legacy scaffold rows are retained; migration does not confer Engine
-- admission or invent missing configuration, original requests, or runtime trust.

-- Bind the runtime profile even before model/repository snapshot registration.
-- This prevents independent clients from reinterpreting a grant-only scope.
ALTER TABLE celetixo.repositories ADD COLUMN authority_profile TEXT
    CHECK (authority_profile IN ('native-reference-v1','source-execution-v1'));
UPDATE celetixo.repositories AS repository
    SET authority_profile = model.runtime_profile
    FROM celetixo.engine_model_heads AS model
    WHERE repository.organization_id = model.organization_id
      AND repository.workspace_id = model.workspace_id
      AND repository.repository_id = model.repository_id
      AND model.runtime_profile IS NOT NULL;
UPDATE celetixo.repositories AS repository SET authority_profile = 'native-reference-v1'
    WHERE repository.authority_profile IS NULL AND EXISTS (
        SELECT 1 FROM celetixo.authority_publications AS publication
        WHERE repository.organization_id = publication.organization_id
          AND repository.workspace_id = publication.workspace_id
          AND repository.repository_id = publication.repository_id);
ALTER TABLE celetixo.repositories ADD CONSTRAINT repository_authority_profile_unique
    UNIQUE (organization_id,workspace_id,repository_id,authority_profile);
CREATE FUNCTION celetixo.preserve_authority_profile() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog,celetixo AS $$
BEGIN
    IF OLD.authority_profile IS NOT NULL
       AND NEW.authority_profile IS DISTINCT FROM OLD.authority_profile THEN
        RAISE EXCEPTION 'repository authority profile is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION celetixo.preserve_authority_profile() FROM PUBLIC;
CREATE TRIGGER preserve_authority_profile BEFORE UPDATE ON celetixo.repositories
    FOR EACH ROW EXECUTE FUNCTION celetixo.preserve_authority_profile();

ALTER TABLE celetixo.engine_model_heads
    DROP CONSTRAINT engine_model_heads_runtime_profile_check,
    DROP CONSTRAINT native_reference_profile_kind;
ALTER TABLE celetixo.engine_model_heads ADD CONSTRAINT engine_model_profile_kind CHECK (
    runtime_profile IS NULL
    OR (runtime_profile = 'native-reference-v1' AND model_kind = 'Reference')
    OR (runtime_profile = 'source-execution-v1' AND model_kind = 'Source'));
ALTER TABLE celetixo.engine_model_heads ADD CONSTRAINT model_repository_profile_fk
    FOREIGN KEY (organization_id,workspace_id,repository_id,runtime_profile)
    REFERENCES celetixo.repositories
        (organization_id,workspace_id,repository_id,authority_profile)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE celetixo.source_analyses (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    analysis_digest celetixo.digest32 NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    index_generation celetixo.digest32 NOT NULL,
    payload_digest celetixo.digest32 NOT NULL CHECK (payload_digest = analysis_digest),
    PRIMARY KEY (organization_id,workspace_id,repository_id,analysis_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_policy_catalog (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    policy_digest celetixo.digest32 NOT NULL,
    policy_kind TEXT NOT NULL DEFAULT 'source_execution' CHECK (policy_kind = 'source_execution'),
    policy_generation celetixo.uint64 NOT NULL CHECK (policy_generation > 0),
    analysis_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,policy_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,policy_kind,policy_generation,policy_digest)
        REFERENCES celetixo.policies
        (organization_id,workspace_id,repository_id,policy_kind,generation,policy_digest)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,analysis_digest)
        REFERENCES celetixo.source_analyses ON DELETE RESTRICT
);
-- The full policy graph includes immutable patch policy, check definitions and
-- exact check specs. A head or definition address alone cannot reconstruct it.
CREATE TABLE celetixo.source_check_definitions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    definition_digest celetixo.digest32 NOT NULL,
    check_spec_digest celetixo.digest32 NOT NULL,
    payload_digest celetixo.digest32 NOT NULL CHECK (payload_digest = definition_digest),
    PRIMARY KEY (organization_id,workspace_id,repository_id,definition_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,check_spec_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_policy_dependencies (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    policy_digest celetixo.digest32 NOT NULL,
    dependency_kind TEXT NOT NULL CHECK (dependency_kind IN ('PatchPolicy','CheckDefinition','CheckSpec')),
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,policy_digest,dependency_kind,payload_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,policy_digest)
        REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_attempt_contexts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    analysis_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number),
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,analysis_digest)
        REFERENCES celetixo.source_analyses ON DELETE RESTRICT
);
-- Policy is selected by successful analysis, after proposal. Absence remains
-- meaningful; adding this immutable association does not rewrite proposal facts.
CREATE TABLE celetixo.source_attempt_policies (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    policy_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number),
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number)
        REFERENCES celetixo.source_attempt_contexts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,policy_digest)
        REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT
);
ALTER TABLE celetixo.source_attempts ADD CONSTRAINT source_attempt_transaction_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id);
CREATE TABLE celetixo.source_admission_contexts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    request_payload_digest celetixo.digest32 NOT NULL,
    admission_fence celetixo.uint64 NOT NULL CHECK (admission_fence > 0),
    lease_event_count celetixo.uint64 NOT NULL CHECK (lease_event_count > 0),
    PRIMARY KEY (organization_id,workspace_id,repository_id,source_attempt_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id)
        REFERENCES celetixo.source_attempts
        (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,admission_fence)
        REFERENCES celetixo.reservations
        (organization_id,workspace_id,repository_id,transaction_id,fence_generation)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,request_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
ALTER TABLE celetixo.operations ADD CONSTRAINT operation_transaction_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,transaction_id);
CREATE TABLE celetixo.source_operation_contexts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    operation_kind TEXT NOT NULL DEFAULT 'SourceAdmission' CHECK (operation_kind = 'SourceAdmission'),
    idempotency_key celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    owner_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (owner_grant_kind = 'session'),
    owner_grant_id celetixo.uint64 NOT NULL CHECK (owner_grant_id > 0),
    recorded_at_ms celetixo.uint64 NOT NULL,
    fence_generation celetixo.uint64 CHECK (fence_generation > 0),
    lease_event_count celetixo.uint64 NOT NULL,
    observed_analysis celetixo.digest32,
    observed_policy celetixo.digest32,
    observed_model_generation celetixo.uint64 NOT NULL CHECK (observed_model_generation > 0),
    PRIMARY KEY (organization_id,workspace_id,repository_id,idempotency_key),
    FOREIGN KEY (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,transaction_id)
        REFERENCES celetixo.operations
        (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,transaction_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,owner_grant_kind,owner_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_analysis)
        REFERENCES celetixo.source_analyses ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_policy)
        REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,fence_generation)
        REFERENCES celetixo.reservations
        (organization_id,workspace_id,repository_id,transaction_id,fence_generation)
        ON DELETE RESTRICT
);

-- The old FK proved that every existing requested ordinal also names a real
-- attempt. Retain it verbatim as submitted data, backfill a separate resolved
-- selector, then constrain only that selector. New rejected requests may name 999.
-- A separate relation avoids updating any immutable historical rejection row.
ALTER TABLE celetixo.source_rejections ADD CONSTRAINT rejection_transaction_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id);
CREATE TABLE celetixo.source_rejection_contexts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    resolved_attempt celetixo.uint64 NOT NULL,
    operation_kind TEXT CHECK (operation_kind = 'SourceAdmission'),
    idempotency_key celetixo.identity,
    PRIMARY KEY (organization_id,workspace_id,repository_id,source_attempt_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id)
        REFERENCES celetixo.source_rejections
        (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,resolved_attempt)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,transaction_id)
        REFERENCES celetixo.operations
        (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,transaction_id)
        ON DELETE RESTRICT,
    CHECK ((operation_kind IS NULL) = (idempotency_key IS NULL))
);
INSERT INTO celetixo.source_rejection_contexts
    (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,resolved_attempt)
    SELECT organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,transaction_attempt
    FROM celetixo.source_rejections;
DO $$
DECLARE constraint_name name;
BEGIN
    SELECT conname INTO STRICT constraint_name FROM pg_catalog.pg_constraint
        WHERE conrelid = 'celetixo.source_rejections'::regclass
          AND confrelid = 'celetixo.attempts'::regclass AND contype = 'f';
    EXECUTE format('ALTER TABLE celetixo.source_rejections DROP CONSTRAINT %I',constraint_name);
END;
$$;
ALTER TABLE celetixo.source_rejections ADD CONSTRAINT source_rejection_context_required_fk
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id)
    REFERENCES celetixo.source_rejection_contexts
    DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE celetixo.source_rejection_events (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    event_sequence celetixo.uint64 NOT NULL,
    at_ms celetixo.uint64 NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence),
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id)
        REFERENCES celetixo.source_rejections ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_unprepared_commits (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    outcome_payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,idempotency_key),
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,outcome_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

-- Configured registrations carry the exact non-secret selector/configuration
-- definition. NULL retains legacy scaffold metadata; it cannot select a durable
-- source adapter without explicit Engine registration and trusted resolution.
ALTER TABLE celetixo.artifact_store_registrations
    ADD COLUMN definition_payload_digest celetixo.digest32;
ALTER TABLE celetixo.artifact_store_registrations ADD CONSTRAINT artifact_store_definition_fk
    FOREIGN KEY (organization_id,workspace_id,repository_id,definition_payload_digest)
    REFERENCES celetixo.domain_payloads ON DELETE RESTRICT;

-- Executor and cleanup definitions are typed normalized rows. Active status and
-- publication generation remain in grants; immutable definitions do not encode
-- revocation as a different identity or manufacture missing issuance history.
ALTER TABLE celetixo.grants ADD CONSTRAINT grant_expiry_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,grant_kind,grant_id,expires_at_ms);
CREATE TABLE celetixo.source_executor_definitions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    grant_id celetixo.uint64 NOT NULL CHECK (grant_id > 0),
    grant_kind TEXT NOT NULL DEFAULT 'executor' CHECK (grant_kind = 'executor'),
    expires_at_ms celetixo.uint64 NOT NULL CHECK (expires_at_ms > 0),
    backend_name TEXT NOT NULL CHECK (
        octet_length(backend_name) BETWEEN 1 AND 4096 AND backend_name !~ '[[:cntrl:]]'),
    backend_version TEXT NOT NULL CHECK (
        octet_length(backend_version) BETWEEN 1 AND 4096 AND backend_version !~ '[[:cntrl:]]'),
    backend_image celetixo.digest32 NOT NULL,
    backend_profile celetixo.digest32 NOT NULL,
    wall_time_ms celetixo.uint64 NOT NULL CHECK (wall_time_ms > 0),
    cpu_time_ms celetixo.uint64 NOT NULL CHECK (cpu_time_ms > 0),
    memory_bytes celetixo.uint64 NOT NULL CHECK (memory_bytes > 0),
    disk_bytes celetixo.uint64 NOT NULL CHECK (disk_bytes > 0),
    process_count celetixo.uint64 NOT NULL CHECK (process_count > 0),
    file_descriptors celetixo.uint64 NOT NULL CHECK (file_descriptors > 0),
    stdout_bytes celetixo.uint64 NOT NULL CHECK (stdout_bytes > 0),
    stderr_bytes celetixo.uint64 NOT NULL CHECK (stderr_bytes > 0),
    artifact_bytes celetixo.uint64 NOT NULL CHECK (artifact_bytes > 0),
    PRIMARY KEY (organization_id,workspace_id,repository_id,grant_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,grant_kind,grant_id,expires_at_ms)
        REFERENCES celetixo.grants
        (organization_id,workspace_id,repository_id,grant_kind,grant_id,expires_at_ms)
        ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_executor_tiers (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    grant_id celetixo.uint64 NOT NULL CHECK (grant_id > 0),
    execution_tier TEXT NOT NULL CHECK (execution_tier IN ('T1','T2','T3')),
    PRIMARY KEY (organization_id,workspace_id,repository_id,grant_id,execution_tier),
    FOREIGN KEY (organization_id,workspace_id,repository_id,grant_id)
        REFERENCES celetixo.source_executor_definitions ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_cleanup_definitions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    grant_id celetixo.uint64 NOT NULL CHECK (grant_id > 0),
    grant_kind TEXT NOT NULL DEFAULT 'cleanup' CHECK (grant_kind = 'cleanup'),
    expires_at_ms celetixo.uint64 NOT NULL CHECK (expires_at_ms > 0),
    backend_profile celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,grant_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,grant_kind,grant_id,expires_at_ms)
        REFERENCES celetixo.grants
        (organization_id,workspace_id,repository_id,grant_kind,grant_id,expires_at_ms)
        ON DELETE RESTRICT
);

-- Physical bytes, per-registration locations and logical names/media are distinct
-- identities. Identical bytes can be retained in different stores and referenced
-- with different names or media without rewriting immutable metadata.
CREATE TABLE celetixo.artifact_contents (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    object_digest celetixo.digest32 NOT NULL,
    byte_length celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,object_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT
);
CREATE TABLE celetixo.artifact_locations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    registration_id celetixo.uint64 NOT NULL CHECK (registration_id > 0),
    object_digest celetixo.digest32 NOT NULL,
    locator_kind TEXT NOT NULL CHECK (locator_kind IN ('LegacyPayload','ContentAddressed')),
    locator_payload_digest celetixo.digest32,
    PRIMARY KEY (organization_id,workspace_id,repository_id,registration_id,object_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,registration_id)
        REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,object_digest)
        REFERENCES celetixo.artifact_contents ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,locator_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    CHECK ((locator_kind = 'LegacyPayload') = (locator_payload_digest IS NOT NULL))
);
CREATE TABLE celetixo.artifact_references (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    registration_id celetixo.uint64 NOT NULL CHECK (registration_id > 0),
    reference_digest celetixo.digest32 NOT NULL,
    object_digest celetixo.digest32 NOT NULL,
    reference_name TEXT NOT NULL CHECK (
        octet_length(reference_name) BETWEEN 1 AND 4096 AND reference_name !~ '[[:cntrl:]]'),
    media_type TEXT NOT NULL CHECK (
        octet_length(media_type) BETWEEN 1 AND 1024 AND media_type !~ '[[:cntrl:]]'),
    retention_digest celetixo.digest32 NOT NULL,
    payload_digest celetixo.digest32 NOT NULL CHECK (payload_digest = reference_digest),
    PRIMARY KEY (organization_id,workspace_id,repository_id,registration_id,reference_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,registration_id,object_digest)
        REFERENCES celetixo.artifact_locations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
-- Preserve all original rows and their old associations. Backfill metadata only;
-- no physical upload, object read, retention confirmation or admission is implied.
INSERT INTO celetixo.artifact_contents
    (organization_id,workspace_id,repository_id,object_digest,byte_length)
    SELECT organization_id,workspace_id,repository_id,object_digest,byte_length
    FROM celetixo.artifact_objects;
INSERT INTO celetixo.artifact_locations
    (organization_id,workspace_id,repository_id,registration_id,object_digest,locator_kind,locator_payload_digest)
    SELECT organization_id,workspace_id,repository_id,registration_id,object_digest,'LegacyPayload',locator_payload_digest
    FROM celetixo.artifact_objects;
DO $$
DECLARE constraint_name name;
BEGIN
    SELECT conname INTO STRICT constraint_name FROM pg_catalog.pg_constraint
        WHERE conrelid = 'celetixo.evidence_receipts'::regclass
          AND confrelid = 'celetixo.artifact_objects'::regclass AND contype = 'f';
    EXECUTE format('ALTER TABLE celetixo.evidence_receipts DROP CONSTRAINT %I',constraint_name);
END;
$$;
ALTER TABLE celetixo.evidence_receipts ADD CONSTRAINT evidence_envelope_content_fk
    FOREIGN KEY (organization_id,workspace_id,repository_id,envelope_object_digest)
    REFERENCES celetixo.artifact_contents ON DELETE RESTRICT;
ALTER TABLE celetixo.evidence_receipts ADD COLUMN envelope_reference_digest celetixo.digest32;
ALTER TABLE celetixo.evidence_receipts ADD CONSTRAINT evidence_envelope_reference_fk
    FOREIGN KEY (organization_id,workspace_id,repository_id,registration_id,envelope_reference_digest)
    REFERENCES celetixo.artifact_references ON DELETE RESTRICT;
CREATE TABLE celetixo.source_evidence_objects (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    evidence_kind TEXT NOT NULL DEFAULT 'SourceV2' CHECK (evidence_kind = 'SourceV2'),
    object_sequence celetixo.uint64 NOT NULL,
    object_role TEXT NOT NULL CHECK (object_role IN ('Envelope','Provenance','Check')),
    check_id celetixo.identity,
    reference_name TEXT NOT NULL CHECK (
        octet_length(reference_name) BETWEEN 1 AND 4096 AND reference_name !~ '[[:cntrl:]]'),
    registration_id celetixo.uint64 NOT NULL CHECK (registration_id > 0),
    reference_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,evidence_digest,object_sequence),
    UNIQUE NULLS NOT DISTINCT
        (organization_id,workspace_id,repository_id,evidence_digest,object_role,check_id,reference_name),
    FOREIGN KEY (organization_id,workspace_id,repository_id,evidence_digest,evidence_kind)
        REFERENCES celetixo.evidence
        (organization_id,workspace_id,repository_id,evidence_digest,evidence_kind)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,registration_id,reference_digest)
        REFERENCES celetixo.artifact_references ON DELETE RESTRICT,
    CHECK ((object_role = 'Check') = (check_id IS NOT NULL))
);

-- Source executor/cleanup claims remain attribution. The authenticated SQL
-- service role/OID stays separate, and Rust checks every opaque capability.
ALTER TABLE celetixo.authority_publications
    DROP CONSTRAINT authority_publications_actor_kind_check,
    DROP CONSTRAINT authority_publications_check;
ALTER TABLE celetixo.authority_publications ADD CONSTRAINT authority_actor_kind_check
    CHECK (actor_kind IN ('Session','EmbeddingAdministrator','SourceExecutor','SourceCleanup'));
ALTER TABLE celetixo.authority_publications ADD CONSTRAINT authority_actor_claims_check CHECK (
    (actor_kind = 'Session' AND requested_principal_id IS NOT NULL
     AND requested_grant_id IS NOT NULL AND requested_grant_id > 0 AND requested_permission IS NOT NULL)
    OR (actor_kind = 'EmbeddingAdministrator' AND requested_principal_id IS NULL
     AND requested_grant_id IS NULL AND requested_permission IS NULL)
    OR (actor_kind IN ('SourceExecutor','SourceCleanup') AND requested_principal_id IS NOT NULL
     AND requested_grant_id IS NOT NULL AND requested_grant_id > 0 AND requested_permission IS NULL));

-- All added fact tables use the same independently authenticated scope boundary.
-- Immutable facts cannot be rewritten by runtime or administrative service roles.
DO $$
DECLARE target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY[
        'source_analyses','source_policy_catalog','source_check_definitions','source_policy_dependencies',
        'source_attempt_contexts','source_attempt_policies','source_admission_contexts','source_operation_contexts',
        'source_rejection_contexts','source_rejection_events','source_unprepared_commits',
        'source_executor_definitions','source_executor_tiers','source_cleanup_definitions',
        'artifact_contents','artifact_locations','artifact_references','source_evidence_objects'
    ] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY',target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY',target);
        EXECUTE format(
            'CREATE POLICY scope_isolation ON celetixo.%I '
            'USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id)) '
            'WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))',target);
        EXECUTE format(
            'CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.%I '
            'FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change()',target);
        EXECUTE format('REVOKE ALL ON celetixo.%I FROM PUBLIC',target);
    END LOOP;
END;
$$;

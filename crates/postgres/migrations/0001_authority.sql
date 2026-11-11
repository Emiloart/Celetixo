-- Goal 04.1: PostgreSQL authority schema. No Engine transition/commit procedure.
-- The migration runner owns celetixo.schema_migrations and its advisory lock.
-- Apply this entire file in the runner's single, explicit database transaction.
CREATE SCHEMA IF NOT EXISTS celetixo;
REVOKE ALL ON SCHEMA celetixo FROM PUBLIC;

CREATE DOMAIN celetixo.uint64 AS NUMERIC(20, 0)
    CHECK (VALUE >= 0 AND VALUE <= 18446744073709551615);
CREATE DOMAIN celetixo.digest32 AS BYTEA
    CHECK (octet_length(VALUE) = 32);
CREATE DOMAIN celetixo.identity AS TEXT
    CHECK (octet_length(VALUE) BETWEEN 1 AND 256
           AND VALUE !~ '[[:space:][:cntrl:]]');
CREATE DOMAIN celetixo.payload_format AS TEXT
    CHECK (octet_length(VALUE) BETWEEN 1 AND 128
           AND VALUE !~ '[[:space:][:cntrl:]]');

-- Deliberately not a runtime table: only the migration/security administrator
-- may insert mappings. Map both role name and OID: drop/recreate of a login with
-- the same name must not inherit a previous login's scope. No FK to a scoped
-- row: a trusted administrator must be able to authorize initial scope creation.
CREATE TABLE celetixo.db_role_scopes (
    database_role NAME NOT NULL,
    database_role_oid OID NOT NULL,
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    authorized_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (database_role_oid, organization_id, workspace_id, repository_id),
    UNIQUE (database_role, organization_id, workspace_id, repository_id)
);

-- session_user is PostgreSQL's independently authenticated login; SET ROLE and
-- client-set tenant GUCs cannot alter it. Fixed search_path and qualified names
-- prevent caller-owned schema/object substitution in these narrow helpers.
-- The authenticated function-owner login is explicitly trusted for schema/data
-- bootstrap under FORCE RLS. Runtime roles must never own these functions or
-- hold membership/SET SESSION AUTHORIZATION authority over the owner role.
CREATE FUNCTION celetixo.has_repository_scope(
    requested_organization TEXT, requested_workspace TEXT, requested_repository TEXT
) RETURNS BOOLEAN
LANGUAGE SQL STABLE SECURITY DEFINER SET search_path = pg_catalog
AS $function$
    SELECT session_user = current_user OR EXISTS (
        SELECT 1 FROM celetixo.db_role_scopes AS mapping
        JOIN pg_catalog.pg_roles AS login
          ON login.oid = mapping.database_role_oid
         AND login.rolname = mapping.database_role
        WHERE mapping.database_role = session_user::name
          AND mapping.organization_id = requested_organization
          AND mapping.workspace_id = requested_workspace
          AND mapping.repository_id = requested_repository
    )
$function$;

CREATE FUNCTION celetixo.has_workspace_scope(
    requested_organization TEXT, requested_workspace TEXT
) RETURNS BOOLEAN
LANGUAGE SQL STABLE SECURITY DEFINER SET search_path = pg_catalog
AS $function$
    SELECT session_user = current_user OR EXISTS (
        SELECT 1 FROM celetixo.db_role_scopes AS mapping
        JOIN pg_catalog.pg_roles AS login
          ON login.oid = mapping.database_role_oid
         AND login.rolname = mapping.database_role
        WHERE mapping.database_role = session_user::name
          AND mapping.organization_id = requested_organization
          AND mapping.workspace_id = requested_workspace
    )
$function$;

CREATE FUNCTION celetixo.has_organization_scope(requested_organization TEXT)
RETURNS BOOLEAN
LANGUAGE SQL STABLE SECURITY DEFINER SET search_path = pg_catalog
AS $function$
    SELECT session_user = current_user OR EXISTS (
        SELECT 1 FROM celetixo.db_role_scopes AS mapping
        JOIN pg_catalog.pg_roles AS login
          ON login.oid = mapping.database_role_oid
         AND login.rolname = mapping.database_role
        WHERE mapping.database_role = session_user::name
          AND mapping.organization_id = requested_organization
    )
$function$;
REVOKE ALL ON FUNCTION celetixo.has_repository_scope(TEXT, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION celetixo.has_workspace_scope(TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION celetixo.has_organization_scope(TEXT) FROM PUBLIC;

CREATE TABLE celetixo.organizations (
    organization_id celetixo.identity PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE celetixo.principals (
    organization_id celetixo.identity NOT NULL,
    principal_id celetixo.identity NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, principal_id),
    FOREIGN KEY (organization_id) REFERENCES celetixo.organizations ON DELETE RESTRICT
);
CREATE TABLE celetixo.workspaces (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id),
    FOREIGN KEY (organization_id) REFERENCES celetixo.organizations ON DELETE RESTRICT
);
CREATE TABLE celetixo.repositories (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    state_generation celetixo.uint64 NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id),
    FOREIGN KEY (organization_id, workspace_id)
        REFERENCES celetixo.workspaces ON DELETE RESTRICT
);

-- Immutable typed values; payload_digest is a domain-separated Engine/storage
-- digest, not a SQL-computed arbitrary SHA. Rust must verify digest, version,
-- exact length and checked domain construction on every load. This is not a
-- serialized Database, an authorization capability or a competing wire schema.
CREATE TABLE celetixo.domain_payloads (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    payload_digest celetixo.digest32 NOT NULL,
    payload_format celetixo.payload_format NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version > 0),
    payload_bytes BYTEA NOT NULL CHECK (octet_length(payload_bytes) <= 67108864),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, payload_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT
);
CREATE TABLE celetixo.snapshots (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    revision celetixo.uint64 NOT NULL,
    manifest_digest celetixo.digest32 NOT NULL,
    payload_digest celetixo.digest32 NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, snapshot_digest),
    UNIQUE (organization_id, workspace_id, repository_id, snapshot_digest, revision),
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.repository_heads (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    revision celetixo.uint64 NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    head_generation celetixo.uint64 NOT NULL DEFAULT 0,
    PRIMARY KEY (organization_id, workspace_id, repository_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, snapshot_digest, revision)
        REFERENCES celetixo.snapshots
        (organization_id, workspace_id, repository_id, snapshot_digest, revision)
        ON DELETE RESTRICT
);

-- grant_id is scoped attribution. No raw bearer credential is stored here.
CREATE TABLE celetixo.grants (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    grant_kind TEXT NOT NULL CHECK (grant_kind IN ('session', 'executor', 'cleanup')),
    grant_id celetixo.uint64 NOT NULL CHECK (grant_id > 0),
    principal_id celetixo.identity NOT NULL,
    generation celetixo.uint64 NOT NULL CHECK (generation > 0),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at_ms celetixo.uint64,
    payload_digest celetixo.digest32,
    PRIMARY KEY (organization_id, workspace_id, repository_id, grant_kind, grant_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, principal_id)
        REFERENCES celetixo.principals ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.grant_permissions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    grant_kind TEXT NOT NULL,
    grant_id celetixo.uint64 NOT NULL,
    permission TEXT NOT NULL CHECK (permission IN (
        'Register', 'ManageModel', 'Read', 'Propose', 'Analyze',
        'Reserve', 'Execute', 'Verify', 'Commit', 'Cancel')),
    PRIMARY KEY (
        organization_id, workspace_id, repository_id, grant_kind, grant_id, permission),
    FOREIGN KEY (organization_id, workspace_id, repository_id, grant_kind, grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT
);
CREATE TABLE celetixo.policies (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    policy_kind TEXT NOT NULL CHECK (policy_kind IN (
        'model', 'source_execution', 'authorization', 'retention')),
    generation celetixo.uint64 NOT NULL CHECK (generation > 0),
    policy_digest celetixo.digest32 NOT NULL,
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, policy_kind, generation),
    UNIQUE (
        organization_id, workspace_id, repository_id, policy_kind, generation, policy_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.policy_heads (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    policy_kind TEXT NOT NULL,
    generation celetixo.uint64 NOT NULL,
    policy_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, policy_kind),
    FOREIGN KEY (
        organization_id, workspace_id, repository_id, policy_kind, generation, policy_digest)
        REFERENCES celetixo.policies
        (organization_id, workspace_id, repository_id, policy_kind, generation, policy_digest)
        ON DELETE RESTRICT
);
CREATE TABLE celetixo.artifact_store_registrations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    registration_id celetixo.uint64 NOT NULL CHECK (registration_id > 0),
    generation celetixo.uint64 NOT NULL CHECK (generation > 0),
    adapter_identity celetixo.identity NOT NULL,
    retention_digest celetixo.digest32 NOT NULL,
    provenance_digest celetixo.digest32 NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (organization_id, workspace_id, repository_id, registration_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT
);

CREATE TABLE celetixo.transactions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    owner_principal_id celetixo.identity NOT NULL,
    owner_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (owner_grant_kind = 'session'),
    owner_grant_id celetixo.uint64 NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'Proposed', 'Analyzed', 'Reserved', 'Executing', 'Validating',
        'RevalidationRequired', 'Committed', 'Aborted')),
    current_attempt celetixo.uint64 NOT NULL CHECK (current_attempt > 0),
    original_snapshot_digest celetixo.digest32 NOT NULL,
    intent_payload_digest celetixo.digest32 NOT NULL,
    generation celetixo.uint64 NOT NULL DEFAULT 0,
    PRIMARY KEY (organization_id, workspace_id, repository_id, transaction_id),
    FOREIGN KEY (organization_id, owner_principal_id)
        REFERENCES celetixo.principals ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, owner_grant_kind, owner_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, original_snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, intent_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.attempts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL CHECK (attempt_number > 0),
    base_snapshot_digest celetixo.digest32 NOT NULL,
    candidate_snapshot_digest celetixo.digest32,
    model_payload_digest celetixo.digest32 NOT NULL,
    analysis_payload_digest celetixo.digest32,
    patch_digest celetixo.digest32,
    patch_payload_digest celetixo.digest32,
    PRIMARY KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id)
        REFERENCES celetixo.transactions ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, base_snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, candidate_snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, model_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, analysis_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, patch_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    CHECK ((patch_digest IS NULL) = (patch_payload_digest IS NULL))
);
-- Initial transaction + attempt are created atomically; this one deferred FK
-- permits either insertion order and is checked before the SQL transaction commits.
ALTER TABLE celetixo.transactions ADD CONSTRAINT transactions_current_attempt_fk
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, current_attempt)
    REFERENCES celetixo.attempts
    (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE celetixo.access_provenance (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    provenance_kind TEXT NOT NULL CHECK (provenance_kind IN (
        'explicit', 'derived', 'effective', 'invariant', 'predicate', 'analysis')),
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (
        organization_id, workspace_id, repository_id,
        transaction_id, attempt_number, provenance_kind),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.transaction_outcomes (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    terminal_state TEXT NOT NULL CHECK (terminal_state IN ('Committed', 'Aborted')),
    outcome_payload_digest celetixo.digest32 NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, transaction_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, outcome_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.committed_changes (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    revision celetixo.uint64 NOT NULL CHECK (revision > 0),
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    patch_digest celetixo.digest32 NOT NULL,
    provenance_payload_digest celetixo.digest32 NOT NULL,
    receipt_payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, revision),
    UNIQUE (organization_id, workspace_id, repository_id, transaction_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, snapshot_digest, revision)
        REFERENCES celetixo.snapshots
        (organization_id, workspace_id, repository_id, snapshot_digest, revision)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, provenance_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, receipt_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

-- One durable fence counter per repository; Engine acquisition increments it
-- atomically and supplies the exact generation for every protected operation.
CREATE TABLE celetixo.reservation_fences (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    generation celetixo.uint64 NOT NULL DEFAULT 0,
    PRIMARY KEY (organization_id, workspace_id, repository_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT
);
CREATE TABLE celetixo.reservations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    fence_generation celetixo.uint64 NOT NULL CHECK (fence_generation > 0),
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    owner_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (owner_grant_kind = 'session'),
    owner_grant_id celetixo.uint64 NOT NULL,
    acquired_at_ms celetixo.uint64 NOT NULL,
    expires_at_ms celetixo.uint64 NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('Active', 'Released', 'Expired', 'Revoked', 'Superseded')),
    PRIMARY KEY (organization_id, workspace_id, repository_id, fence_generation),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, owner_grant_kind, owner_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    CHECK (expires_at_ms > acquired_at_ms)
);
CREATE TABLE celetixo.reservation_claims (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    element_id celetixo.identity NOT NULL,
    fence_generation celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, element_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, fence_generation)
        REFERENCES celetixo.reservations ON DELETE RESTRICT
);
CREATE TABLE celetixo.reservation_events (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    fence_generation celetixo.uint64 NOT NULL,
    event_sequence celetixo.uint64 NOT NULL,
    at_ms celetixo.uint64 NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN (
        'Acquired', 'Renewed', 'Expired', 'Revoked', 'Released', 'Superseded')),
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (
        organization_id, workspace_id, repository_id, fence_generation, event_sequence),
    FOREIGN KEY (organization_id, workspace_id, repository_id, fence_generation)
        REFERENCES celetixo.reservations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

-- Insert only a resolved immutable outcome. Request claims/in-flight work live
-- in transactions/attempts; an uncommitted operation is never a guessed outcome.
CREATE TABLE celetixo.operations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind IN (
        'Commit', 'SourceAdmission', 'EvidenceAdmission', 'Reservation', 'Cancellation')),
    idempotency_key celetixo.identity NOT NULL,
    request_digest celetixo.digest32 NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    outcome_kind TEXT NOT NULL CHECK (outcome_kind IN ('Accepted', 'Rejected')),
    outcome_payload_digest celetixo.digest32 NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (
        organization_id, workspace_id, repository_id, operation_kind, idempotency_key),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, outcome_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

CREATE TABLE celetixo.source_attempts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    transaction_attempt celetixo.uint64 NOT NULL,
    admission_digest celetixo.digest32 NOT NULL,
    admission_payload_digest celetixo.digest32 NOT NULL,
    patch_digest celetixo.digest32 NOT NULL,
    patch_payload_digest celetixo.digest32 NOT NULL,
    application_payload_digest celetixo.digest32 NOT NULL,
    owner_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (owner_grant_kind = 'session'),
    owner_grant_id celetixo.uint64 NOT NULL,
    executor_grant_kind TEXT NOT NULL DEFAULT 'executor' CHECK (executor_grant_kind = 'executor'),
    executor_grant_id celetixo.uint64,
    state TEXT NOT NULL CHECK (state IN (
        'Proposed', 'Admitted', 'Patching', 'PatchApplied', 'Verifying',
        'EvidenceSealed', 'Rejected', 'PatchConflict', 'ExecutionFailed',
        'TimedOut', 'Cancelled', 'EvidenceIncomplete', 'RevalidationRequired')),
    PRIMARY KEY (organization_id, workspace_id, repository_id, source_attempt_id),
    UNIQUE (organization_id, workspace_id, repository_id, admission_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, transaction_attempt)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, admission_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, patch_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, application_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, owner_grant_kind, owner_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, executor_grant_kind, executor_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT
);
CREATE UNIQUE INDEX source_attempts_one_active_per_transaction ON celetixo.source_attempts
    (organization_id, workspace_id, repository_id, transaction_id)
    WHERE state IN ('Proposed', 'Admitted', 'Patching', 'PatchApplied', 'Verifying');
CREATE TABLE celetixo.source_attempt_events (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    event_sequence celetixo.uint64 NOT NULL,
    at_ms celetixo.uint64 NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (
        organization_id, workspace_id, repository_id, source_attempt_id, event_sequence),
    FOREIGN KEY (organization_id, workspace_id, repository_id, source_attempt_id)
        REFERENCES celetixo.source_attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.source_attempt_outcomes (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    terminal_state TEXT NOT NULL CHECK (terminal_state IN (
        'EvidenceSealed', 'Rejected', 'PatchConflict', 'ExecutionFailed',
        'TimedOut', 'Cancelled', 'EvidenceIncomplete', 'RevalidationRequired')),
    payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, source_attempt_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, source_attempt_id)
        REFERENCES celetixo.source_attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
-- Rejection may occur before an executable admission exists. It therefore has
-- its own immutable record, without inventing a source_attempt/admission row.
CREATE TABLE celetixo.source_rejections (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    transaction_attempt celetixo.uint64 NOT NULL,
    request_digest celetixo.digest32 NOT NULL,
    rejection_payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, source_attempt_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, transaction_attempt)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, rejection_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

-- Object metadata is not proof of upload/durability. Engine independently
-- verifies object bytes before it atomically inserts an admission receipt.
CREATE TABLE celetixo.artifact_objects (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    object_digest celetixo.digest32 NOT NULL,
    byte_length celetixo.uint64 NOT NULL,
    media_type TEXT NOT NULL CHECK (octet_length(media_type) BETWEEN 1 AND 256),
    registration_id celetixo.uint64 NOT NULL,
    locator_payload_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, object_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id, registration_id)
        REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, locator_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.evidence (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    evidence_kind TEXT NOT NULL CHECK (evidence_kind IN ('Reference', 'SourceV2')),
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    evidence_payload_digest celetixo.digest32 NOT NULL,
    verifier_grant_kind TEXT NOT NULL,
    verifier_grant_id celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, evidence_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, evidence_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, verifier_grant_kind, verifier_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    CHECK (
        (evidence_kind = 'Reference' AND verifier_grant_kind = 'session')
        OR (evidence_kind = 'SourceV2' AND verifier_grant_kind = 'executor'))
);
CREATE TABLE celetixo.evidence_receipts (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    registration_id celetixo.uint64 NOT NULL,
    assessment TEXT NOT NULL CHECK (assessment IN ('Passed', 'Failed', 'Incomplete')),
    envelope_object_digest celetixo.digest32 NOT NULL,
    receipt_payload_digest celetixo.digest32 NOT NULL,
    recorded_at_ms celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, evidence_digest),
    UNIQUE (organization_id, workspace_id, repository_id, source_attempt_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, evidence_digest)
        REFERENCES celetixo.evidence ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, source_attempt_id)
        REFERENCES celetixo.source_attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, registration_id)
        REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, envelope_object_digest)
        REFERENCES celetixo.artifact_objects ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, receipt_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.attempt_checks (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    check_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number, check_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, attempt_number)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, evidence_digest)
        REFERENCES celetixo.evidence ON DELETE RESTRICT
);

-- Immutable event facts and independently mutable publisher delivery state.
-- Neither an outbox record nor an inbox delivery is transaction authority.
CREATE TABLE celetixo.outbox (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    event_id celetixo.digest32 NOT NULL,
    aggregate_kind TEXT NOT NULL CHECK (aggregate_kind IN ('Repository', 'Transaction', 'SourceAttempt')),
    aggregate_id celetixo.identity NOT NULL,
    aggregate_sequence celetixo.uint64 NOT NULL,
    event_type celetixo.identity NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    payload_digest celetixo.digest32 NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, event_id),
    UNIQUE (
        organization_id, workspace_id, repository_id,
        aggregate_kind, aggregate_id, aggregate_sequence),
    FOREIGN KEY (organization_id, workspace_id, repository_id, payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
CREATE TABLE celetixo.outbox_delivery (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    event_id celetixo.digest32 NOT NULL,
    publication_attempts celetixo.uint64 NOT NULL DEFAULT 0,
    published_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ,
    claim_generation celetixo.uint64 NOT NULL DEFAULT 0,
    PRIMARY KEY (organization_id, workspace_id, repository_id, event_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, event_id)
        REFERENCES celetixo.outbox ON DELETE RESTRICT
);
CREATE INDEX outbox_delivery_pending ON celetixo.outbox_delivery
    (next_attempt_at, organization_id, workspace_id, repository_id)
    WHERE published_at IS NULL;
CREATE TABLE celetixo.inbox (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    consumer_id celetixo.identity NOT NULL,
    event_id celetixo.digest32 NOT NULL,
    event_payload_digest celetixo.digest32 NOT NULL,
    result_payload_digest celetixo.digest32 NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, consumer_id, event_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, event_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, result_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);

CREATE FUNCTION celetixo.reject_immutable_change()
RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog
AS $function$
BEGIN
    RAISE EXCEPTION 'immutable authority record cannot be changed: %', TG_TABLE_NAME
        USING ERRCODE = '23514';
END
$function$;
REVOKE ALL ON FUNCTION celetixo.reject_immutable_change() FROM PUBLIC;

-- Trusted migration/admin roles can change DDL; ordinary runtime roles cannot.
-- No cascade deletes or table-wide runtime grants are introduced here.
DO $install$
DECLARE
    target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY[
        'domain_payloads', 'snapshots', 'policies', 'access_provenance',
        'transaction_outcomes', 'committed_changes', 'reservation_events',
        'operations', 'source_attempt_events', 'source_attempt_outcomes',
        'source_rejections', 'artifact_objects', 'evidence', 'evidence_receipts',
        'outbox', 'inbox'
    ] LOOP
        EXECUTE format(
            'CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.%I '
            'FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change()', target);
    END LOOP;

    FOREACH target IN ARRAY ARRAY[
        'repositories', 'domain_payloads', 'snapshots', 'repository_heads',
        'grants', 'grant_permissions', 'policies', 'policy_heads',
        'artifact_store_registrations', 'transactions', 'attempts',
        'access_provenance', 'transaction_outcomes', 'committed_changes',
        'reservation_fences', 'reservations', 'reservation_claims', 'reservation_events',
        'operations', 'source_attempts', 'source_attempt_events', 'source_attempt_outcomes',
        'source_rejections', 'artifact_objects', 'evidence', 'evidence_receipts',
        'attempt_checks', 'outbox', 'outbox_delivery', 'inbox'
    ] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY', target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY', target);
        EXECUTE format(
            'CREATE POLICY scope_isolation ON celetixo.%I '
            'USING (celetixo.has_repository_scope(organization_id, workspace_id, repository_id)) '
            'WITH CHECK (celetixo.has_repository_scope(organization_id, workspace_id, repository_id))',
            target);
    END LOOP;

    FOREACH target IN ARRAY ARRAY['organizations', 'principals'] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY', target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY', target);
        EXECUTE format(
            'CREATE POLICY scope_isolation ON celetixo.%I '
            'USING (celetixo.has_organization_scope(organization_id)) '
            'WITH CHECK (celetixo.has_organization_scope(organization_id))', target);
    END LOOP;
END
$install$;
ALTER TABLE celetixo.workspaces ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.workspaces FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.workspaces
    USING (celetixo.has_workspace_scope(organization_id, workspace_id))
    WITH CHECK (celetixo.has_workspace_scope(organization_id, workspace_id));

-- Fail closed until the separately authenticated deployment administrator grants
-- a reviewed per-table privilege set to a nonowner, non-BYPASSRLS runtime login.
REVOKE ALL ON ALL TABLES IN SCHEMA celetixo FROM PUBLIC;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA celetixo FROM PUBLIC;

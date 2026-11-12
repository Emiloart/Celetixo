-- Additive reconstruction metadata. Published schema v1 is immutable.
-- Rust attempts are zero-based; retain that identity rather than translating it.
ALTER TABLE celetixo.transactions DROP CONSTRAINT transactions_current_attempt_check;
ALTER TABLE celetixo.attempts DROP CONSTRAINT attempts_attempt_number_check;
ALTER TABLE celetixo.transactions ADD COLUMN last_reason TEXT
    CHECK (last_reason IS NULL OR octet_length(last_reason) <= 67108864);

-- Retain the original request body as well as its semantic digest. Current
-- attempt/evidence state may have advanced before a lost-response retry.
ALTER TABLE celetixo.operations ADD COLUMN request_payload_digest celetixo.digest32;
ALTER TABLE celetixo.operations ADD CONSTRAINT operations_request_payload_fk
    FOREIGN KEY (organization_id, workspace_id, repository_id, request_payload_digest)
    REFERENCES celetixo.domain_payloads ON DELETE RESTRICT;

CREATE TABLE celetixo.engine_counters (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    last_clock_ms celetixo.uint64 NOT NULL DEFAULT 0,
    next_session_grant celetixo.uint64 NOT NULL DEFAULT 0,
    next_executor_grant celetixo.uint64 NOT NULL DEFAULT 0,
    next_cleanup_grant celetixo.uint64 NOT NULL DEFAULT 0,
    next_artifact_store celetixo.uint64 NOT NULL DEFAULT 0,
    PRIMARY KEY (organization_id, workspace_id, repository_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT
);

-- A model head binds to the same current immutable snapshot as repository head.
-- Deferred checking permits their atomic update in either order within a single
-- SQL transaction. A candidate payload alone cannot install a model head.
ALTER TABLE celetixo.repository_heads ADD CONSTRAINT repository_heads_snapshot_unique
    UNIQUE (organization_id, workspace_id, repository_id, snapshot_digest);
CREATE TABLE celetixo.engine_model_heads (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    initial_snapshot_digest celetixo.digest32 NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    model_generation celetixo.uint64 NOT NULL CHECK (model_generation > 0),
    model_payload_digest celetixo.digest32 NOT NULL,
    model_kind TEXT NOT NULL CHECK (model_kind IN ('Reference', 'Source')),
    analysis_payload_digest celetixo.digest32,
    index_generation celetixo.digest32,
    PRIMARY KEY (organization_id, workspace_id, repository_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, initial_snapshot_digest)
        REFERENCES celetixo.snapshots
        (organization_id, workspace_id, repository_id, snapshot_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, snapshot_digest)
        REFERENCES celetixo.repository_heads
        (organization_id, workspace_id, repository_id, snapshot_digest)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (organization_id, workspace_id, repository_id, model_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, analysis_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    CHECK (
        (model_kind = 'Reference' AND analysis_payload_digest IS NULL AND index_generation IS NULL)
        OR (model_kind = 'Source' AND analysis_payload_digest IS NOT NULL AND index_generation IS NOT NULL)
    )
);

ALTER TABLE celetixo.engine_counters ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.engine_counters FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.engine_counters
    USING (celetixo.has_repository_scope(organization_id, workspace_id, repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id, workspace_id, repository_id));
ALTER TABLE celetixo.engine_model_heads ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.engine_model_heads FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.engine_model_heads
    USING (celetixo.has_repository_scope(organization_id, workspace_id, repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id, workspace_id, repository_id));
REVOKE ALL ON celetixo.engine_counters, celetixo.engine_model_heads FROM PUBLIC;

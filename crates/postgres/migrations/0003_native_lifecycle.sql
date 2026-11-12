-- Native Engine lifecycle integration metadata. Published v1/v2 are immutable.
-- Legacy rows remain preserved; no claim of Engine admission is inferred from
-- their shape. Only an explicit Rust registration installs this runtime profile.
ALTER TABLE celetixo.engine_model_heads ADD COLUMN runtime_profile TEXT
    CHECK (runtime_profile IS NULL OR runtime_profile = 'native-reference-v1');
ALTER TABLE celetixo.engine_model_heads ADD CONSTRAINT native_reference_profile_kind
    CHECK (runtime_profile IS NULL OR model_kind = 'Reference');
ALTER TABLE celetixo.attempts ADD COLUMN model_generation celetixo.uint64
    CHECK (model_generation IS NULL OR model_generation > 0);

-- A retained lease may have been acquired for an earlier attempt before fresh
-- revalidation. Keep its original reservation row and an explicit current pointer.
ALTER TABLE celetixo.reservations ADD CONSTRAINT reservations_owner_fence_unique
    UNIQUE (organization_id, workspace_id, repository_id, transaction_id, fence_generation);
ALTER TABLE celetixo.transactions ADD COLUMN current_fence celetixo.uint64
    CHECK (current_fence IS NULL OR current_fence > 0);
ALTER TABLE celetixo.transactions ADD CONSTRAINT transactions_current_fence_fk
    FOREIGN KEY (organization_id, workspace_id, repository_id, transaction_id, current_fence)
    REFERENCES celetixo.reservations
    (organization_id, workspace_id, repository_id, transaction_id, fence_generation)
    DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE celetixo.transactions ADD CONSTRAINT terminal_transactions_release_fence
    CHECK (state NOT IN ('Committed','Aborted') OR current_fence IS NULL);

-- Native strings can contain NUL; SQL TEXT cannot. New native-profile writers
-- retain this checked binary record, leaving legacy last_reason as NULL. Legacy
-- text is preserved for explicitly reviewed future migration, never sanitized.
ALTER TABLE celetixo.transactions ADD COLUMN last_reason_payload_digest celetixo.digest32;
ALTER TABLE celetixo.transactions ADD CONSTRAINT transaction_reason_payload_fk
    FOREIGN KEY (organization_id, workspace_id, repository_id, last_reason_payload_digest)
    REFERENCES celetixo.domain_payloads ON DELETE RESTRICT;

-- An evidence value binds its verifier principal. A new grant for that principal
-- may freshly admit identical bytes through the Rust verifier gate. Preserve the
-- immutable original issuer, append admissions and select the current signer;
-- never rewrite an immutable evidence record to change its historical issuer.
ALTER TABLE celetixo.evidence ADD CONSTRAINT evidence_kind_unique
    UNIQUE (organization_id, workspace_id, repository_id, evidence_digest, evidence_kind);
CREATE TABLE celetixo.reference_evidence_admissions (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    evidence_kind TEXT NOT NULL DEFAULT 'Reference' CHECK (evidence_kind = 'Reference'),
    verifier_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (verifier_grant_kind = 'session'),
    verifier_grant_id celetixo.uint64 NOT NULL,
    admitted_at_ms celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, evidence_digest, verifier_grant_id),
    FOREIGN KEY (organization_id, workspace_id, repository_id, evidence_digest, evidence_kind)
        REFERENCES celetixo.evidence
        (organization_id, workspace_id, repository_id, evidence_digest, evidence_kind) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, verifier_grant_kind, verifier_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT
);
CREATE TABLE celetixo.reference_evidence_signers (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    verifier_grant_id celetixo.uint64 NOT NULL,
    PRIMARY KEY (organization_id, workspace_id, repository_id, evidence_digest),
    FOREIGN KEY (organization_id, workspace_id, repository_id, evidence_digest, verifier_grant_id)
        REFERENCES celetixo.reference_evidence_admissions ON DELETE RESTRICT
);

-- Authenticated database service identity is separate from requested actor
-- attribution. These claims do not grant permission; Rust still checks the opaque
-- Session and current scoped grant. Runtime and administrative writes share the
-- repository lock, generation CAS and this append-only publication audit.
CREATE TABLE celetixo.authority_publications (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    state_generation celetixo.uint64 NOT NULL CHECK (state_generation > 0),
    observed_clock_ms celetixo.uint64 NOT NULL,
    authenticated_database_role NAME NOT NULL,
    authenticated_database_role_oid OID NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('Session','EmbeddingAdministrator')),
    requested_principal_id celetixo.identity,
    requested_grant_id celetixo.uint64,
    requested_permission TEXT CHECK (requested_permission IN (
        'Register','ManageModel','Read','Propose','Analyze','Reserve','Execute','Verify','Commit','Cancel')),
    snapshot_digest celetixo.digest32,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, workspace_id, repository_id, state_generation),
    FOREIGN KEY (organization_id, workspace_id, repository_id)
        REFERENCES celetixo.repositories ON DELETE RESTRICT,
    FOREIGN KEY (organization_id, workspace_id, repository_id, snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    CHECK (
        (actor_kind = 'Session' AND requested_principal_id IS NOT NULL
         AND requested_grant_id IS NOT NULL AND requested_grant_id > 0 AND requested_permission IS NOT NULL)
        OR (actor_kind = 'EmbeddingAdministrator' AND requested_principal_id IS NULL
         AND requested_grant_id IS NULL AND requested_permission IS NULL)
    )
);

ALTER TABLE celetixo.reference_evidence_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.reference_evidence_admissions FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.reference_evidence_admissions
    USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id));
ALTER TABLE celetixo.reference_evidence_signers ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.reference_evidence_signers FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.reference_evidence_signers
    USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id));
ALTER TABLE celetixo.authority_publications ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.authority_publications FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.authority_publications
    USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id));
CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.reference_evidence_admissions
    FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change();
CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.authority_publications
    FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change();
REVOKE ALL ON celetixo.reference_evidence_admissions,celetixo.reference_evidence_signers,celetixo.authority_publications FROM PUBLIC;

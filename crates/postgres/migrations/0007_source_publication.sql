-- Add source-specific publication to the same original-outcome journal.
-- Migrations 0001-0006 and their existing row meanings remain immutable.
ALTER TABLE celetixo.engine_mutations DROP CONSTRAINT mutation_request_kind,
    ADD CONSTRAINT mutation_request_kind CHECK (request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose','Analyze','Reserve','Renew','Execute','Abort','SubmitPatch','RecordEvidence','Revalidate','ClaimSourceAttempt','SourcePatchApplied','BeginSourceVerification','FinishSourceAttempt','CommitSource')),
    DROP CONSTRAINT mutation_result_kind,
    ADD CONSTRAINT mutation_result_kind CHECK (result_kind IN ('Allocation','Revocation','Policy','Repository','Lease','State','Analysis','Bindings','Evidence','SourceAttempt','SourceCommitted'));

-- Preserve the complete checked v6 expression, then add exactly one accepted
-- alternative. The ledger fixes the prerequisite definition; no old alternative
-- is removed, relaxed or rewritten by this extension.
DO $$
DECLARE previous_expression TEXT;
BEGIN
    SELECT pg_catalog.pg_get_expr(conbin,conrelid) INTO STRICT previous_expression
        FROM pg_catalog.pg_constraint
        WHERE conrelid='celetixo.engine_mutations'::regclass AND conname='mutation_outcome_shape';
    ALTER TABLE celetixo.engine_mutations DROP CONSTRAINT mutation_outcome_shape;
    EXECUTE format('ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT mutation_outcome_shape CHECK ((%s) OR ((actor_kind = ''Session'' AND request_kind = ''CommitSource'' AND outcome_kind = ''Accepted'' AND result_kind IS NOT DISTINCT FROM ''SourceCommitted'' AND state_before IS NOT DISTINCT FROM ''Validating'' AND resolved_attempt IS NOT NULL AND lease_events_before > 0 AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL AND result_acquisition_attempt IS NULL AND result_state IS NULL AND result_attempt IS NULL AND result_access_digest IS NULL) IS TRUE))', previous_expression);
END;
$$;
ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT source_commit_journal_identity
    UNIQUE (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest);
ALTER TABLE celetixo.evidence_receipts ADD CONSTRAINT source_commit_evidence_identity
    UNIQUE (organization_id,workspace_id,repository_id,evidence_digest,source_attempt_id,registration_id);

CREATE TABLE celetixo.source_commit_mutations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    actor_digest celetixo.digest32 NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    request_kind TEXT NOT NULL DEFAULT 'CommitSource' CHECK (request_kind='CommitSource'),
    request_digest celetixo.digest32 NOT NULL,
    analysis_grant_kind TEXT NOT NULL DEFAULT 'session' CHECK (analysis_grant_kind='session'),
    analysis_principal celetixo.identity NOT NULL,
    analysis_grant celetixo.uint64 NOT NULL CHECK (analysis_grant > 0),
    observed_head celetixo.digest32,
    observed_model_generation celetixo.uint64 CHECK (observed_model_generation > 0),
    observed_analysis celetixo.digest32,
    observed_policy celetixo.digest32,
    source_attempt_id celetixo.identity,
    admission_digest celetixo.digest32,
    source_state TEXT,
    source_event_count celetixo.uint64,
    source_event_sequence celetixo.uint64,
    evidence_digest celetixo.digest32,
    executor_grant_kind TEXT NOT NULL DEFAULT 'executor' CHECK (executor_grant_kind='executor'),
    executor_grant celetixo.uint64 CHECK (executor_grant > 0),
    executor_active BOOLEAN,
    store_registration celetixo.uint64 CHECK (store_registration > 0),
    store_active BOOLEAN,
    artifact_evidence celetixo.digest32,
    artifact_registration celetixo.uint64 CHECK (artifact_registration > 0),
    artifact_provenance celetixo.digest32,
    artifact_observed_at_ms celetixo.uint64,
    receipt_digest celetixo.digest32,
    PRIMARY KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key),
    UNIQUE (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,receipt_digest),
    FOREIGN KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest)
        REFERENCES celetixo.engine_mutations (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,analysis_grant_kind,analysis_grant,analysis_principal)
        REFERENCES celetixo.grants (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_head) REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_analysis) REFERENCES celetixo.source_analyses ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_policy) REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest)
        REFERENCES celetixo.source_attempts (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,source_event_sequence,source_state)
        REFERENCES celetixo.source_attempt_events (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,to_state) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,evidence_digest) REFERENCES celetixo.evidence ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,executor_grant)
        REFERENCES celetixo.source_attempts (organization_id,workspace_id,repository_id,source_attempt_id,executor_grant_id) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,executor_grant_kind,executor_grant) REFERENCES celetixo.grants ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,store_registration) REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,evidence_digest,source_attempt_id,store_registration)
        REFERENCES celetixo.evidence_receipts (organization_id,workspace_id,repository_id,evidence_digest,source_attempt_id,registration_id) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,artifact_evidence) REFERENCES celetixo.evidence ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,artifact_registration) REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,receipt_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    CHECK (((observed_head IS NULL AND observed_model_generation IS NULL AND observed_analysis IS NULL AND observed_policy IS NULL)
        OR (observed_head IS NOT NULL AND observed_model_generation IS NOT NULL AND observed_analysis IS NOT NULL)) IS TRUE),
    CHECK (((source_attempt_id IS NULL AND admission_digest IS NULL AND source_state IS NULL AND source_event_count IS NULL AND source_event_sequence IS NULL AND evidence_digest IS NULL AND executor_grant IS NULL AND executor_active IS NULL AND store_registration IS NULL AND store_active IS NULL)
        OR (source_attempt_id IS NOT NULL AND admission_digest IS NOT NULL AND source_state IS NOT NULL AND source_event_count > 0 AND source_event_sequence IS NOT NULL AND source_event_sequence=source_event_count-1)) IS TRUE),
    CHECK ((executor_grant IS NULL) = (executor_active IS NULL)),
    CHECK ((store_registration IS NULL) = (store_active IS NULL)),
    CHECK (source_state IS DISTINCT FROM 'EvidenceSealed' OR
        (evidence_digest IS NOT NULL AND executor_grant IS NOT NULL AND store_registration IS NOT NULL)),
    CHECK (((artifact_evidence IS NULL AND artifact_registration IS NULL AND artifact_provenance IS NULL AND artifact_observed_at_ms IS NULL)
        OR (artifact_evidence IS NOT NULL AND artifact_registration IS NOT NULL AND artifact_provenance IS NOT NULL AND artifact_observed_at_ms IS NOT NULL AND source_state='EvidenceSealed' AND evidence_digest=artifact_evidence AND store_registration=artifact_registration)) IS TRUE),
    CHECK (receipt_digest IS NULL OR (artifact_evidence IS NOT NULL AND executor_active IS TRUE AND store_active IS TRUE))
);

-- Require the normalized context for every new source journal row at COMMIT.
ALTER TABLE celetixo.engine_mutations ADD COLUMN source_commit_actor_digest celetixo.digest32
    GENERATED ALWAYS AS (CASE WHEN request_kind='CommitSource' THEN actor_digest END) STORED;
ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT source_commit_context_required
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_commit_actor_digest,idempotency_key)
    REFERENCES celetixo.source_commit_mutations DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE celetixo.source_committed_changes (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    revision celetixo.uint64 NOT NULL CHECK (revision > 0),
    transaction_id celetixo.identity NOT NULL,
    attempt_number celetixo.uint64 NOT NULL,
    snapshot_digest celetixo.digest32 NOT NULL,
    verified_snapshot_digest celetixo.digest32 NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    admission_digest celetixo.digest32 NOT NULL,
    source_patch_digest celetixo.digest32 NOT NULL,
    evidence_digest celetixo.digest32 NOT NULL,
    analysis_digest celetixo.digest32 NOT NULL,
    model_generation celetixo.uint64 NOT NULL CHECK (model_generation > 0),
    committed_at_ms celetixo.uint64 NOT NULL,
    actor_digest celetixo.digest32 NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    receipt_digest celetixo.digest32 NOT NULL,
    accesses_digest celetixo.digest32 NOT NULL,
    provenance_digest celetixo.digest32 NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,revision),
    UNIQUE (organization_id,workspace_id,repository_id,receipt_digest),
    UNIQUE (organization_id,workspace_id,repository_id,transaction_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,attempt_number) REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,snapshot_digest) REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest)
        REFERENCES celetixo.source_attempts (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,evidence_digest) REFERENCES celetixo.evidence ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,analysis_digest) REFERENCES celetixo.source_analyses ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,receipt_digest)
        REFERENCES celetixo.source_commit_mutations (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,receipt_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,receipt_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,accesses_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,provenance_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT
);
ALTER TABLE celetixo.source_commit_mutations ADD CONSTRAINT source_commit_effect_required
    FOREIGN KEY (organization_id,workspace_id,repository_id,receipt_digest)
    REFERENCES celetixo.source_committed_changes (organization_id,workspace_id,repository_id,receipt_digest)
    DEFERRABLE INITIALLY DEFERRED;

DO $$
DECLARE target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY['source_commit_mutations','source_committed_changes'] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY',target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY',target);
        EXECUTE format('CREATE POLICY scope_isolation ON celetixo.%I USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id)) WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))',target);
        EXECUTE format('CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.%I FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change()',target);
        EXECUTE format('REVOKE ALL ON celetixo.%I FROM PUBLIC',target);
    END LOOP;
END;
$$;

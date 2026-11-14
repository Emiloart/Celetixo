-- Add a fresh captured-source epoch to the existing original-outcome journal.
-- The seven published migrations and their row meanings remain immutable.
ALTER TABLE celetixo.engine_mutations DROP CONSTRAINT mutation_request_kind,
    ADD CONSTRAINT mutation_request_kind CHECK (request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose','Analyze','Reserve','Renew','Execute','Abort','SubmitPatch','RecordEvidence','Revalidate','ClaimSourceAttempt','SourcePatchApplied','BeginSourceVerification','FinishSourceAttempt','CommitSource','RevalidateSource')),
    DROP CONSTRAINT mutation_result_kind,
    ADD CONSTRAINT mutation_result_kind CHECK (result_kind IN ('Allocation','Revocation','Policy','Repository','Lease','State','Analysis','Bindings','Evidence','SourceAttempt','SourceCommitted','SourceRevalidated'));
DO $$
DECLARE previous_expression TEXT;
BEGIN
    SELECT pg_catalog.pg_get_expr(conbin,conrelid) INTO STRICT previous_expression
        FROM pg_catalog.pg_constraint WHERE conrelid='celetixo.engine_mutations'::regclass AND conname='mutation_outcome_shape';
    ALTER TABLE celetixo.engine_mutations DROP CONSTRAINT mutation_outcome_shape;
    EXECUTE format('ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT mutation_outcome_shape CHECK ((%s) OR ((actor_kind = ''Session'' AND request_kind = ''RevalidateSource'' AND outcome_kind = ''Accepted'' AND result_kind IS NOT DISTINCT FROM ''SourceRevalidated'' AND state_before IN (''Executing'',''Validating'',''RevalidationRequired'') AND resolved_attempt IS NOT NULL AND lease_events_before > 0 AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL AND result_acquisition_attempt IS NULL AND result_state IS NULL AND result_attempt IS NULL AND result_access_digest IS NULL) IS TRUE))',previous_expression);
END;
$$;
ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT source_revalidation_outcome_identity UNIQUE (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest,outcome_kind);
CREATE TABLE celetixo.source_revalidation_mutations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    actor_digest celetixo.digest32 NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    request_kind TEXT NOT NULL DEFAULT 'RevalidateSource' CHECK (request_kind='RevalidateSource'),
    request_digest celetixo.digest32 NOT NULL,
    outcome_kind TEXT NOT NULL CHECK (outcome_kind IN ('Accepted','Rejected')),
    observed_head celetixo.digest32,
    observed_model_generation celetixo.uint64 CHECK (observed_model_generation>0),
    observed_analysis celetixo.digest32,
    observed_policy celetixo.digest32,
    observed_fence celetixo.uint64 NOT NULL,
    observed_lease_count celetixo.uint64 NOT NULL,
    observed_source_count celetixo.uint64 NOT NULL,
    result_transaction celetixo.identity,
    result_attempt celetixo.uint64 CHECK (result_attempt>0),
    result_fence celetixo.uint64 CHECK (result_fence>0),
    result_owner_grant celetixo.uint64 CHECK (result_owner_grant>0),
    result_expiry_ms celetixo.uint64,
    result_accesses celetixo.digest32,
    result_impact celetixo.digest32,
    receipt_digest celetixo.digest32,
    PRIMARY KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key),
    UNIQUE (organization_id,workspace_id,repository_id,result_transaction,result_attempt),
    FOREIGN KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest,outcome_kind)
        REFERENCES celetixo.engine_mutations (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,request_kind,request_digest,outcome_kind) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_head) REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_analysis) REFERENCES celetixo.source_analyses ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,observed_policy) REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_transaction,result_attempt) REFERENCES celetixo.source_attempt_contexts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_transaction,result_fence,result_attempt,result_owner_grant)
        REFERENCES celetixo.reservations (organization_id,workspace_id,repository_id,transaction_id,fence_generation,attempt_number,owner_grant_id) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_accesses) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,receipt_digest) REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    CHECK (((observed_head IS NULL AND observed_model_generation IS NULL AND observed_analysis IS NULL AND observed_policy IS NULL)
        OR (observed_head IS NOT NULL AND observed_model_generation IS NOT NULL AND observed_analysis IS NOT NULL)) IS TRUE),
    CHECK ((receipt_digest IS NOT NULL) = (outcome_kind='Accepted')),
    CHECK (((receipt_digest IS NULL AND result_transaction IS NULL AND result_attempt IS NULL AND result_fence IS NULL AND result_owner_grant IS NULL AND result_expiry_ms IS NULL AND result_accesses IS NULL AND result_impact IS NULL)
        OR (receipt_digest IS NOT NULL AND result_transaction IS NOT NULL AND result_attempt IS NOT NULL AND result_fence IS NOT NULL AND result_owner_grant IS NOT NULL AND result_expiry_ms IS NOT NULL AND result_accesses IS NOT NULL AND result_impact IS NOT NULL AND result_fence=observed_fence+1)) IS TRUE)
);
ALTER TABLE celetixo.engine_mutations ADD COLUMN source_revalidation_actor_digest celetixo.digest32
    GENERATED ALWAYS AS (CASE WHEN request_kind='RevalidateSource' THEN actor_digest END) STORED;
ALTER TABLE celetixo.engine_mutations ADD CONSTRAINT source_revalidation_context_required
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_revalidation_actor_digest,idempotency_key)
    REFERENCES celetixo.source_revalidation_mutations DEFERRABLE INITIALLY DEFERRED;
-- Every noninitial source epoch has exactly one original accepted revalidation.
ALTER TABLE celetixo.source_attempt_contexts ADD COLUMN revalidation_attempt celetixo.uint64
    GENERATED ALWAYS AS (CASE WHEN attempt_number>0 THEN attempt_number END) STORED;
ALTER TABLE celetixo.source_attempt_contexts ADD CONSTRAINT source_epoch_revalidation_required
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,revalidation_attempt)
    REFERENCES celetixo.source_revalidation_mutations (organization_id,workspace_id,repository_id,result_transaction,result_attempt)
    DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE celetixo.source_revalidation_leases (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    actor_digest celetixo.digest32 NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    transaction_id celetixo.identity NOT NULL,
    event_count celetixo.uint64 NOT NULL,
    grant_active BOOLEAN,
    last_fence celetixo.uint64,
    last_event celetixo.uint64,
    PRIMARY KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,transaction_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key) REFERENCES celetixo.source_revalidation_mutations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id) REFERENCES celetixo.transactions ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,last_fence,last_event) REFERENCES celetixo.reservation_events ON DELETE RESTRICT,
    CHECK (((event_count=0 AND last_fence IS NULL AND last_event IS NULL AND grant_active IS NULL)
        OR (event_count>0 AND last_fence IS NOT NULL AND last_event IS NOT NULL AND last_event=event_count-1)) IS TRUE)
);
CREATE TABLE celetixo.source_revalidation_sources (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    actor_digest celetixo.digest32 NOT NULL,
    idempotency_key celetixo.identity NOT NULL,
    source_attempt_id celetixo.identity NOT NULL,
    event_count celetixo.uint64 NOT NULL CHECK (event_count>0),
    event_sequence celetixo.uint64 NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key,source_attempt_id),
    FOREIGN KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key) REFERENCES celetixo.source_revalidation_mutations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,state)
        REFERENCES celetixo.source_attempt_events (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,to_state) ON DELETE RESTRICT,
    CHECK (event_sequence=event_count-1)
);
DO $$
DECLARE target TEXT;
BEGIN
    FOREACH target IN ARRAY ARRAY['source_revalidation_mutations','source_revalidation_leases','source_revalidation_sources'] LOOP
        EXECUTE format('ALTER TABLE celetixo.%I ENABLE ROW LEVEL SECURITY',target);
        EXECUTE format('ALTER TABLE celetixo.%I FORCE ROW LEVEL SECURITY',target);
        EXECUTE format('CREATE POLICY scope_isolation ON celetixo.%I USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id)) WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))',target);
        EXECUTE format('CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.%I FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change()',target);
        EXECUTE format('REVOKE ALL ON celetixo.%I FROM PUBLIC',target);
    END LOOP;
END;
$$;

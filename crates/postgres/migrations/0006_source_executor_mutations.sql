-- Extend the same immutable original-outcome journal. Published migrations
-- 0001-0005 and existing rows retain their original bytes and meaning.
ALTER TABLE celetixo.source_attempts ADD CONSTRAINT source_mutation_admission_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest);
ALTER TABLE celetixo.source_attempts ADD CONSTRAINT source_mutation_executor_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,executor_grant_id);
ALTER TABLE celetixo.source_attempt_events ADD CONSTRAINT source_mutation_event_state_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,to_state);
ALTER TABLE celetixo.source_attempt_events ADD CONSTRAINT source_mutation_event_time_unique
    UNIQUE (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,at_ms);

ALTER TABLE celetixo.engine_mutations
    ADD COLUMN raw_source_attempt_id celetixo.identity,
    ADD COLUMN raw_source_admission_digest celetixo.digest32,
    ADD COLUMN resolved_source_attempt_id celetixo.identity,
    ADD COLUMN source_admission_digest celetixo.digest32,
    ADD COLUMN source_state_before TEXT,
    ADD COLUMN source_executor_before celetixo.uint64 CHECK (source_executor_before > 0),
    ADD COLUMN source_events_before celetixo.uint64,
    ADD COLUMN source_events_after celetixo.uint64,
    ADD COLUMN source_before_event_sequence celetixo.uint64,
    ADD COLUMN source_effect_event_sequence celetixo.uint64,
    ADD COLUMN result_source_state TEXT,
    ADD COLUMN result_source_event_sequence celetixo.uint64,
    ADD COLUMN result_source_executor_grant_id celetixo.uint64 CHECK (result_source_executor_grant_id > 0);

-- Names are those established by the immutable 0005 CREATE TABLE. Replace only
-- its actor/request/result alternatives; all other constraints, immutable-row
-- trigger, scoped RLS and administrator privilege policy remain in force.
ALTER TABLE celetixo.engine_mutations
    DROP CONSTRAINT engine_mutations_actor_kind_check,
    DROP CONSTRAINT engine_mutations_grant_kind_check,
    DROP CONSTRAINT engine_mutations_request_kind_check,
    DROP CONSTRAINT engine_mutations_result_kind_check,
    DROP CONSTRAINT engine_mutations_check1,
    ADD CONSTRAINT mutation_actor_kind CHECK (actor_kind IN ('Session','EmbeddingAdministrator','SourceExecutor')),
    ADD CONSTRAINT mutation_grant_kind CHECK (grant_kind IN ('session','executor')),
    ADD CONSTRAINT mutation_request_kind CHECK (request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose','Analyze','Reserve','Renew','Execute','Abort','SubmitPatch','RecordEvidence','Revalidate','ClaimSourceAttempt','SourcePatchApplied','BeginSourceVerification','FinishSourceAttempt')),
    ADD CONSTRAINT mutation_result_kind CHECK (result_kind IN ('Allocation','Revocation','Policy','Repository','Lease','State','Analysis','Bindings','Evidence','SourceAttempt')),
    ADD CONSTRAINT mutation_actor_shape CHECK (
        ((actor_kind IN ('Session','SourceExecutor')
            AND grant_kind IS NOT DISTINCT FROM CASE actor_kind WHEN 'Session' THEN 'session' ELSE 'executor' END
            AND actor_principal_id IS NOT NULL AND actor_grant_id IS NOT NULL
            AND admin_counter_before IS NULL AND admin_target_active_before IS NULL AND administrative_target_id IS NULL)
        OR (actor_kind = 'EmbeddingAdministrator' AND grant_kind IS NULL
            AND actor_principal_id IS NULL AND actor_grant_id IS NULL AND admin_counter_before IS NOT NULL)) IS TRUE),
    ADD CONSTRAINT mutation_source_actor_request CHECK (
        (actor_kind = 'SourceExecutor') = (request_kind IN ('ClaimSourceAttempt','SourcePatchApplied','BeginSourceVerification','FinishSourceAttempt')));

ALTER TABLE celetixo.engine_mutations
    ADD CONSTRAINT mutation_source_admission_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_source_attempt_id,source_admission_digest)
        REFERENCES celetixo.source_attempts
        (organization_id,workspace_id,repository_id,source_attempt_id,admission_digest) ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_before_executor_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,grant_kind,source_executor_before)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_before_event_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_source_attempt_id,source_before_event_sequence,source_state_before)
        REFERENCES celetixo.source_attempt_events
        (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,to_state) ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_effect_event_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_source_attempt_id,source_effect_event_sequence,recorded_at_ms)
        REFERENCES celetixo.source_attempt_events
        (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,at_ms) ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_result_event_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_source_attempt_id,result_source_event_sequence,result_source_state)
        REFERENCES celetixo.source_attempt_events
        (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,to_state) ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_result_executor_fk
        FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_source_attempt_id,result_source_executor_grant_id)
        REFERENCES celetixo.source_attempts
        (organization_id,workspace_id,repository_id,source_attempt_id,executor_grant_id) ON DELETE RESTRICT,
    ADD CONSTRAINT mutation_source_context_shape CHECK (
        ((actor_kind <> 'SourceExecutor'
            AND raw_source_attempt_id IS NULL AND raw_source_admission_digest IS NULL
            AND resolved_source_attempt_id IS NULL AND source_admission_digest IS NULL
            AND source_state_before IS NULL AND source_executor_before IS NULL
            AND source_events_before IS NULL AND source_events_after IS NULL
            AND source_before_event_sequence IS NULL AND source_effect_event_sequence IS NULL
            AND result_source_state IS NULL AND result_source_event_sequence IS NULL
            AND result_source_executor_grant_id IS NULL)
        OR (actor_kind = 'SourceExecutor'
            AND raw_source_attempt_id IS NOT NULL AND raw_source_admission_digest IS NOT NULL
            AND source_events_before IS NOT NULL AND source_events_after IS NOT NULL
            AND source_events_after BETWEEN source_events_before AND source_events_before + 1
            AND ((resolved_source_attempt_id IS NULL AND source_admission_digest IS NULL
                    AND source_state_before IS NULL AND source_executor_before IS NULL
                    AND source_events_before = 0 AND source_events_after = 0 AND source_before_event_sequence IS NULL)
                OR (resolved_source_attempt_id IS NOT NULL AND resolved_source_attempt_id = raw_source_attempt_id
                    AND source_admission_digest IS NOT NULL AND source_state_before IS NOT NULL
                    AND source_events_before > 0 AND source_before_event_sequence IS NOT NULL
                    AND source_before_event_sequence = source_events_before - 1))
            AND ((source_events_after = source_events_before AND source_effect_event_sequence IS NULL)
                OR (source_events_after = source_events_before + 1 AND source_effect_event_sequence IS NOT NULL
                    AND source_effect_event_sequence = source_events_before))
            AND ((outcome_kind = 'Rejected' AND result_source_state IS NULL
                    AND result_source_event_sequence IS NULL AND result_source_executor_grant_id IS NULL)
                OR (outcome_kind = 'Accepted' AND result_kind IS NOT DISTINCT FROM 'SourceAttempt'
                    AND resolved_source_attempt_id IS NOT NULL AND source_admission_digest = raw_source_admission_digest
                    AND state_before IS NOT DISTINCT FROM 'Executing' AND resolved_attempt IS NOT NULL
                    AND source_events_after = source_events_before + 1
                    AND result_source_event_sequence IS NOT NULL AND result_source_event_sequence = source_events_before
                    AND result_source_executor_grant_id IS NOT NULL AND result_source_executor_grant_id = actor_grant_id
                    AND result_source_state IS NOT NULL
                    AND ((request_kind = 'ClaimSourceAttempt' AND result_source_state = 'Patching' AND source_executor_before IS NULL)
                        OR (request_kind = 'SourcePatchApplied' AND result_source_state = 'PatchApplied' AND source_executor_before = actor_grant_id)
                        OR (request_kind = 'BeginSourceVerification' AND result_source_state = 'Verifying' AND source_executor_before = actor_grant_id)
                        OR (request_kind = 'FinishSourceAttempt' AND source_executor_before = actor_grant_id
                            AND result_source_state IN ('Rejected','PatchConflict','ExecutionFailed','TimedOut','Cancelled','EvidenceIncomplete','RevalidationRequired'))))))) IS TRUE);

CREATE UNIQUE INDEX mutation_source_event_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,resolved_source_attempt_id,source_effect_event_sequence)
    WHERE source_effect_event_sequence IS NOT NULL;

-- Preserve every v5 outcome alternative and add the separately constrained
-- historical source result. New source metadata is governed above with IS TRUE.
ALTER TABLE celetixo.engine_mutations DROP CONSTRAINT engine_mutations_check11,
    ADD CONSTRAINT mutation_outcome_shape CHECK (((outcome_kind = 'Accepted' AND (resolved_attempt IS NOT NULL OR request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose')) AND result_kind IS NOT NULL
          AND ((result_kind = 'Lease' AND request_kind IN ('Reserve','Renew')
            AND event_sequence IS NOT NULL AND event_sequence = lease_events_before
            AND result_fence IS NOT NULL AND result_expiry_ms IS NOT NULL
            AND result_acquisition_attempt IS NOT NULL
            AND result_expiry_ms > recorded_at_ms AND result_acquisition_attempt <= resolved_attempt
            AND result_state IS NULL AND result_attempt IS NULL AND result_access_digest IS NULL)
          OR (result_kind = 'State' AND result_state IS NOT NULL
            AND result_attempt IS NOT NULL AND result_attempt = COALESCE(resolved_attempt,0)
            AND ((request_kind = 'Execute' AND result_state = 'Executing') OR (request_kind = 'Abort' AND result_state = 'Aborted')
                OR (request_kind = 'Propose' AND result_state = COALESCE(state_before,'Proposed')))
            AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
            AND result_acquisition_attempt IS NULL AND result_access_digest IS NULL)
          OR (result_kind = 'Analysis' AND request_kind = 'Analyze' AND result_state = 'Analyzed'
            AND result_state IS NOT NULL AND result_attempt IS NOT NULL AND result_attempt = resolved_attempt
            AND result_access_digest IS NOT NULL AND state_before = 'Proposed'
            AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
            AND result_acquisition_attempt IS NULL)
          OR (result_kind = 'Bindings' AND request_kind IN ('SubmitPatch','Revalidate') AND result_state = 'Validating'
            AND result_state IS NOT NULL AND result_attempt IS NOT NULL AND resolved_attempt IS NOT NULL
            AND ((request_kind = 'SubmitPatch' AND state_before = 'Executing' AND result_attempt = resolved_attempt)
                OR (request_kind = 'Revalidate' AND state_before IN ('Executing','RevalidationRequired') AND result_attempt = resolved_attempt + 1))
            AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
            AND result_acquisition_attempt IS NULL AND result_access_digest IS NULL)
          OR (result_kind = 'Evidence' AND request_kind = 'RecordEvidence' AND result_state = 'Validating'
            AND state_before = 'Validating'
            AND result_state IS NOT NULL AND result_attempt IS NOT NULL AND result_attempt = resolved_attempt
            AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
            AND result_acquisition_attempt IS NULL AND result_access_digest IS NULL)
          OR (result_kind IN ('Repository','Policy','Allocation','Revocation') AND ((result_kind = 'Repository' AND request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis')) OR (result_kind = 'Policy' AND request_kind = 'InstallPolicy') OR (result_kind = 'Allocation' AND request_kind IN ('GrantSession','GrantExecutor','GrantCleanup','RegisterArtifactStore')) OR (result_kind = 'Revocation' AND request_kind IN ('RevokeSession','RevokeExecutor','RevokeCleanup','RevokeArtifactStore')))
            AND resolved_transaction_id IS NULL AND resolved_attempt IS NULL AND state_before IS NULL
            AND lease_events_before = 0 AND result_state IS NULL AND result_attempt IS NULL
            AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
            AND result_acquisition_attempt IS NULL AND result_access_digest IS NULL)))
        OR (outcome_kind = 'Rejected' AND event_sequence IS NULL AND result_fence IS NULL
            AND result_expiry_ms IS NULL AND result_acquisition_attempt IS NULL
            AND result_kind IS NULL AND result_state IS NULL AND result_attempt IS NULL AND result_access_digest IS NULL)) OR (actor_kind = 'SourceExecutor' AND outcome_kind = 'Accepted'
        AND result_kind IS NOT DISTINCT FROM 'SourceAttempt' AND resolved_attempt IS NOT NULL
        AND state_before IS NOT DISTINCT FROM 'Executing'
        AND event_sequence IS NULL AND result_fence IS NULL AND result_expiry_ms IS NULL
        AND result_acquisition_attempt IS NULL AND result_state IS NULL
        AND result_attempt IS NULL AND result_access_digest IS NULL));

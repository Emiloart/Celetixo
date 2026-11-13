-- Original keyed mutations share the Engine's existing transaction boundary.
-- Published migrations 0001-0004 remain unchanged. Raw rejected selectors are
-- data; only separately resolved targets and produced effects carry foreign keys.
ALTER TABLE celetixo.grants ADD CONSTRAINT grant_principal_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id);
ALTER TABLE celetixo.reservations ADD CONSTRAINT reservation_mutation_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,transaction_id,fence_generation,attempt_number,owner_grant_id);
ALTER TABLE celetixo.attempts ADD CONSTRAINT attempt_result_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,transaction_id,attempt_number,base_snapshot_digest,candidate_snapshot_digest,patch_digest,analysis_payload_digest);
ALTER TABLE celetixo.evidence ADD CONSTRAINT evidence_mutation_identity_unique
    UNIQUE (organization_id,workspace_id,repository_id,transaction_id,attempt_number,evidence_digest);

CREATE TABLE celetixo.engine_mutations (
    organization_id celetixo.identity NOT NULL,
    workspace_id celetixo.identity NOT NULL,
    repository_id celetixo.identity NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('Session','EmbeddingAdministrator')),
    grant_kind TEXT DEFAULT 'session' CHECK (grant_kind = 'session'),
    actor_principal_id celetixo.identity,
    actor_grant_id celetixo.uint64 CHECK (actor_grant_id > 0),
    idempotency_key celetixo.identity NOT NULL,
    request_kind TEXT NOT NULL CHECK (request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose','Analyze','Reserve','Renew','Execute','Abort','SubmitPatch','RecordEvidence','Revalidate')),
    request_digest celetixo.digest32 NOT NULL,
    request_payload_digest celetixo.digest32 NOT NULL CHECK (request_payload_digest = request_digest),
    transaction_id celetixo.identity,
    recorded_at_ms celetixo.uint64 NOT NULL,
    resolved_transaction_id celetixo.identity,
    resolved_attempt celetixo.uint64,
    state_before TEXT CHECK (state_before IN ('Proposed','Analyzed','Reserved','Executing','Validating','RevalidationRequired','Committed','Aborted')),
    lease_events_before celetixo.uint64 NOT NULL,
    outcome_kind TEXT NOT NULL CHECK (outcome_kind IN ('Accepted','Rejected')),
    outcome_payload_digest celetixo.digest32 NOT NULL,
    event_sequence celetixo.uint64,
    result_fence celetixo.uint64 CHECK (result_fence > 0),
    result_expiry_ms celetixo.uint64,
    result_acquisition_attempt celetixo.uint64,
    result_kind TEXT CHECK (result_kind IN ('Allocation','Revocation','Policy','Repository','Lease','State','Analysis','Bindings','Evidence')),
    result_state TEXT CHECK (result_state IN ('Proposed','Analyzed','Reserved','Executing','Validating','RevalidationRequired','Committed','Aborted')),
    result_attempt celetixo.uint64,
    result_access_digest celetixo.digest32,
    result_base_snapshot_digest celetixo.digest32,
    result_candidate_snapshot_digest celetixo.digest32,
    result_patch_digest celetixo.digest32,
    result_analysis_digest celetixo.digest32,
    result_analysis_payload_digest celetixo.digest32,
    result_evidence_digest celetixo.digest32,
    result_repository_snapshot_digest celetixo.digest32,
    result_repository_model_digest celetixo.digest32,
    result_repository_model_generation celetixo.uint64 CHECK (result_repository_model_generation > 0),
    result_policy_digest celetixo.digest32,
    result_policy_model_generation celetixo.uint64 CHECK (result_policy_model_generation > 0),
    actor_digest celetixo.digest32 NOT NULL,
    admin_counter_before celetixo.uint64,
    admin_target_active_before BOOLEAN,
    administrative_target_id celetixo.uint64,
    result_resource_kind TEXT CHECK (result_resource_kind IN ('Session','Executor','Cleanup','ArtifactStore')),
    result_resource_id celetixo.uint64 CHECK (result_resource_id > 0),
    result_grant_kind TEXT CHECK (result_grant_kind IN ('session','executor','cleanup')),
    result_grant_id celetixo.uint64 CHECK (result_grant_id > 0),
    result_store_registration_id celetixo.uint64 CHECK (result_store_registration_id > 0),
    PRIMARY KEY (organization_id,workspace_id,repository_id,actor_digest,idempotency_key),
    FOREIGN KEY (organization_id,workspace_id,repository_id,grant_kind,actor_grant_id,actor_principal_id)
        REFERENCES celetixo.grants
        (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_grant_kind,result_grant_id)
        REFERENCES celetixo.grants ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_store_registration_id)
        REFERENCES celetixo.artifact_store_registrations ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,request_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,outcome_payload_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_repository_snapshot_digest)
        REFERENCES celetixo.snapshots ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_repository_model_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_policy_digest)
        REFERENCES celetixo.source_policy_catalog ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_transaction_id,resolved_attempt)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,result_attempt)
        REFERENCES celetixo.attempts ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_access_digest)
        REFERENCES celetixo.domain_payloads ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,result_attempt,result_base_snapshot_digest,result_candidate_snapshot_digest,result_patch_digest,result_analysis_payload_digest)
        REFERENCES celetixo.attempts
        (organization_id,workspace_id,repository_id,transaction_id,attempt_number,base_snapshot_digest,candidate_snapshot_digest,patch_digest,analysis_payload_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,transaction_id,result_attempt,result_evidence_digest)
        REFERENCES celetixo.evidence
        (organization_id,workspace_id,repository_id,transaction_id,attempt_number,evidence_digest) ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_evidence_digest,actor_grant_id)
        REFERENCES celetixo.reference_evidence_admissions ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,result_fence,event_sequence)
        REFERENCES celetixo.reservation_events ON DELETE RESTRICT,
    FOREIGN KEY (organization_id,workspace_id,repository_id,resolved_transaction_id,result_fence,result_acquisition_attempt,actor_grant_id)
        REFERENCES celetixo.reservations
        (organization_id,workspace_id,repository_id,transaction_id,fence_generation,attempt_number,owner_grant_id)
        ON DELETE RESTRICT,
    CHECK ((actor_kind = 'Session' AND grant_kind IS NOT DISTINCT FROM 'session' AND actor_principal_id IS NOT NULL AND actor_grant_id IS NOT NULL AND admin_counter_before IS NULL AND admin_target_active_before IS NULL AND administrative_target_id IS NULL)
        OR (actor_kind = 'EmbeddingAdministrator' AND grant_kind IS NULL AND actor_principal_id IS NULL AND actor_grant_id IS NULL AND admin_counter_before IS NOT NULL)),
    CHECK ((actor_kind = 'EmbeddingAdministrator') = (request_kind IN ('GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore'))),
    CHECK ((request_kind IN ('RevokeSession','RevokeExecutor','RevokeCleanup','RevokeArtifactStore')) = (administrative_target_id IS NOT NULL)),
    CHECK (admin_target_active_before IS NULL OR (administrative_target_id IS NOT NULL AND administrative_target_id > 0 AND admin_counter_before IS NOT NULL AND administrative_target_id <= admin_counter_before)),
    CHECK ((resolved_transaction_id IS NULL) = (resolved_attempt IS NULL)),
    CHECK ((transaction_id IS NULL) = (request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore'))),
    CHECK (transaction_id IS NOT NULL OR
        (resolved_transaction_id IS NULL AND resolved_attempt IS NULL AND state_before IS NULL AND lease_events_before = 0)),
    CHECK ((resolved_attempt IS NULL) = (state_before IS NULL)),
    CHECK (resolved_transaction_id IS NULL OR resolved_transaction_id = transaction_id),
    CHECK (resolved_attempt IS NOT NULL OR (lease_events_before = 0 AND (outcome_kind = 'Rejected' OR request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose')))),
    CHECK ((outcome_kind = 'Accepted' AND (resolved_attempt IS NOT NULL OR request_kind IN ('Register','UpdateModel','RegisterRepository','InstallAnalysis','InstallPolicy','GrantSession','RevokeSession','GrantExecutor','RevokeExecutor','GrantCleanup','RevokeCleanup','RegisterArtifactStore','RevokeArtifactStore','Propose')) AND result_kind IS NOT NULL
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
            AND result_kind IS NULL AND result_state IS NULL AND result_attempt IS NULL AND result_access_digest IS NULL)),
    CHECK ((result_kind IS NOT DISTINCT FROM 'Bindings') =
        (result_base_snapshot_digest IS NOT NULL AND result_candidate_snapshot_digest IS NOT NULL
            AND result_patch_digest IS NOT NULL AND result_analysis_digest IS NOT NULL AND result_analysis_payload_digest IS NOT NULL)),
    CHECK (result_kind IS NOT DISTINCT FROM 'Bindings' OR
        (result_base_snapshot_digest IS NULL AND result_candidate_snapshot_digest IS NULL
            AND result_patch_digest IS NULL AND result_analysis_digest IS NULL AND result_analysis_payload_digest IS NULL)),
    CHECK ((result_kind IS NOT DISTINCT FROM 'Evidence') = (result_evidence_digest IS NOT NULL)),
    CHECK ((result_kind IS NOT DISTINCT FROM 'Repository') =
        (result_repository_snapshot_digest IS NOT NULL AND result_repository_model_digest IS NOT NULL AND result_repository_model_generation IS NOT NULL)),
    CHECK (result_kind IS NOT DISTINCT FROM 'Repository' OR
        (result_repository_snapshot_digest IS NULL AND result_repository_model_digest IS NULL AND result_repository_model_generation IS NULL)),
    CHECK ((result_kind IS NOT DISTINCT FROM 'Policy') = (result_policy_digest IS NOT NULL AND result_policy_model_generation IS NOT NULL)),
    CHECK (result_kind IS NOT DISTINCT FROM 'Policy' OR (result_policy_digest IS NULL AND result_policy_model_generation IS NULL)),
    CHECK (((result_kind IS NOT DISTINCT FROM 'Allocation') OR (result_kind IS NOT DISTINCT FROM 'Revocation')) = (result_resource_kind IS NOT NULL AND result_resource_id IS NOT NULL)),
    CHECK ((result_kind IS NOT DISTINCT FROM 'Allocation') OR (result_kind IS NOT DISTINCT FROM 'Revocation') OR (result_resource_kind IS NULL AND result_resource_id IS NULL)),
    CHECK (result_resource_kind IS NULL OR
        ((result_resource_kind = 'Session' AND request_kind IN ('GrantSession','RevokeSession') AND result_grant_kind IS NOT DISTINCT FROM 'session')
        OR (result_resource_kind = 'Executor' AND request_kind IN ('GrantExecutor','RevokeExecutor') AND result_grant_kind IS NOT DISTINCT FROM 'executor')
        OR (result_resource_kind = 'Cleanup' AND request_kind IN ('GrantCleanup','RevokeCleanup') AND result_grant_kind IS NOT DISTINCT FROM 'cleanup')
        OR (result_resource_kind = 'ArtifactStore' AND request_kind IN ('RegisterArtifactStore','RevokeArtifactStore') AND result_grant_kind IS NULL))),
    CHECK ((result_resource_kind IS NOT NULL AND result_resource_kind IN ('Session','Executor','Cleanup') AND result_grant_id IS NOT NULL AND result_grant_id = result_resource_id AND result_store_registration_id IS NULL)
        OR (result_resource_kind IS NOT DISTINCT FROM 'ArtifactStore' AND result_grant_id IS NULL AND result_store_registration_id IS NOT NULL AND result_store_registration_id = result_resource_id)
        OR (result_resource_kind IS NULL AND result_grant_kind IS NULL AND result_grant_id IS NULL AND result_store_registration_id IS NULL)),
    CHECK (result_kind IS DISTINCT FROM 'Allocation' OR (admin_counter_before IS NOT NULL AND result_resource_id = admin_counter_before + 1 AND administrative_target_id IS NULL AND admin_target_active_before IS NULL)),
    CHECK (result_kind IS DISTINCT FROM 'Revocation' OR (administrative_target_id IS NOT NULL AND result_resource_id = administrative_target_id AND admin_target_active_before IS NOT NULL))
);
CREATE UNIQUE INDEX mutation_lease_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,resolved_transaction_id,event_sequence)
    WHERE result_kind = 'Lease';
CREATE UNIQUE INDEX mutation_analysis_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,resolved_transaction_id,result_attempt)
    WHERE result_kind = 'Analysis';
CREATE UNIQUE INDEX mutation_proposal_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,transaction_id)
    WHERE request_kind = 'Propose' AND resolved_attempt IS NULL AND outcome_kind = 'Accepted';
CREATE UNIQUE INDEX mutation_revalidation_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,transaction_id,result_attempt)
    WHERE request_kind = 'Revalidate' AND outcome_kind = 'Accepted';
CREATE UNIQUE INDEX mutation_patch_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,transaction_id,result_attempt)
    WHERE request_kind = 'SubmitPatch' AND outcome_kind = 'Accepted';
CREATE UNIQUE INDEX mutation_registration_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id) WHERE request_kind IN ('Register','RegisterRepository') AND outcome_kind = 'Accepted';
CREATE UNIQUE INDEX mutation_model_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,result_repository_model_generation) WHERE request_kind IN ('Register','UpdateModel','RegisterRepository') AND outcome_kind = 'Accepted';
CREATE UNIQUE INDEX mutation_allocation_effect_unique ON celetixo.engine_mutations
    (organization_id,workspace_id,repository_id,result_resource_kind,result_resource_id) WHERE result_kind = 'Allocation';
ALTER TABLE celetixo.engine_mutations ENABLE ROW LEVEL SECURITY;
ALTER TABLE celetixo.engine_mutations FORCE ROW LEVEL SECURITY;
CREATE POLICY scope_isolation ON celetixo.engine_mutations
    USING (celetixo.has_repository_scope(organization_id,workspace_id,repository_id))
    WITH CHECK (celetixo.has_repository_scope(organization_id,workspace_id,repository_id)
        AND (actor_kind <> 'EmbeddingAdministrator'
            OR CASE
                WHEN request_kind IN ('GrantSession','GrantExecutor','GrantCleanup')
                    THEN pg_catalog.has_table_privilege(session_user,'celetixo.grants','INSERT')
                WHEN request_kind IN ('RevokeSession','RevokeExecutor','RevokeCleanup')
                    THEN pg_catalog.has_column_privilege(session_user,'celetixo.grants','active','UPDATE')
                WHEN request_kind = 'RegisterArtifactStore'
                    THEN pg_catalog.has_table_privilege(session_user,'celetixo.artifact_store_registrations','INSERT')
                WHEN request_kind = 'RevokeArtifactStore'
                    THEN pg_catalog.has_column_privilege(session_user,'celetixo.artifact_store_registrations','active','UPDATE')
                ELSE FALSE END));
CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON celetixo.engine_mutations
    FOR EACH ROW EXECUTE FUNCTION celetixo.reject_immutable_change();
REVOKE ALL ON celetixo.engine_mutations FROM PUBLIC;

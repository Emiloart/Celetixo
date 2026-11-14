use super::*;
use celetixo_engine::access::AccessSet;
use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::snapshot::{FileMode, InputKind, InputLayer, WorkspaceInput};
use celetixo_engine::source_evidence::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use celetixo_execution::artifacts::{ArtifactStoreLimits, LocalSourceArtifactStore};

#[path = "mutations.rs"]
mod mutations;
#[path = "publication.rs"]
mod publication;

fn element() -> ElementId {
    ElementId::new("symbol:main").unwrap()
}
fn check_id() -> CheckId {
    CheckId::new("synthetic-check").unwrap()
}
fn transaction_id() -> TransactionId {
    TransactionId::new("transaction").unwrap()
}
pub(super) struct Harness {
    pub(super) authority: Arc<PostgresAuthority>,
    pub(super) engine: Engine,
    pub(super) admin: Administrator,
    pub(super) owner: Session,
    pub(super) executor: SourceExecutorCapability,
    pub(super) storage: SourceArtifactStoreCapability,
    pub(super) base: Snapshot,
    pub(super) policy: SourceExecutionPolicy,
    pub(super) artifacts: Option<Arc<LocalSourceArtifactStore>>,
    pub(super) artifact_path: PathBuf,
}
impl Harness {
    pub(super) fn prepare(&self) {
        self.prepare_with_ttl(300_000);
    }
    fn prepare_with_ttl(&self, ttl: u64) {
        self.propose();
        self.engine
            .analyze(&self.owner, self.scope(), &transaction_id())
            .unwrap();
        self.engine
            .reserve(&self.owner, self.scope(), &transaction_id(), ttl)
            .unwrap();
        self.engine
            .execute(&self.owner, self.scope(), &transaction_id())
            .unwrap();
    }
    fn artifacts(&self) -> &LocalSourceArtifactStore {
        self.artifacts.as_deref().unwrap()
    }
    pub(super) fn ready(&self, name: &str) -> (SourcePatch, SourceAdmission, AppliedSourcePatch) {
        self.ready_with_ttl(name, 300_000)
    }
    fn ready_with_ttl(
        &self,
        name: &str,
        ttl: u64,
    ) -> (SourcePatch, SourceAdmission, AppliedSourcePatch) {
        self.prepare_with_ttl(ttl);
        let patch = self.patch(name);
        let admission = self
            .engine
            .admit_source_attempt(
                &self.owner,
                self.scope(),
                patch.clone(),
                IdempotencyKey::new(name).unwrap(),
            )
            .unwrap();
        self.engine
            .claim_source_attempt(&self.executor, &admission)
            .unwrap();
        let application = self
            .engine
            .source_application(&self.executor, &admission)
            .unwrap();
        self.engine
            .mark_source_patch_applied(&self.executor, &admission)
            .unwrap();
        self.engine
            .begin_source_verification(&self.executor, &admission)
            .unwrap();
        (patch, admission, application)
    }
    pub(super) fn synthetic_evidence(
        &self,
        patch: &SourcePatch,
        admission: &SourceAdmission,
        application: &AppliedSourcePatch,
        case: u8,
    ) -> (SourceEvidence, SourceArtifactReference) {
        let check = admission.plan().checks()[0].definition().input();
        let started = self
            .engine
            .source_execution_time(&self.executor, admission)
            .unwrap();
        let mut input = SourceCheckRecordInput {
            admission: admission.id(),
            plan: admission.plan().id(),
            candidate: admission.resulting_snapshot(),
            check: check_id(),
            definition: admission.plan().checks()[0].definition().digest(),
            backend: check.backend.digest(),
            executable: check.toolchain.executable_digest,
            runtime_closure: check.toolchain.runtime_closure,
            environment: check.check.environment(),
            limits: check.resources,
            started_at_ms: started,
            finished_at_ms: started,
            outcome: SourceCheckOutcome::Exited {
                code: if case == 1 { 7 } else { 0 },
            },
            usage: SourceResourceUsage {
                wall_time_ms: 0,
                cpu_time_us: 0,
                peak_memory_bytes: 1,
            },
            post_source_manifest: Some(admission.resulting_manifest()),
            termination: SourceTermination::Confirmed,
            cleanup: if case == 2 {
                SourceCleanup::Quarantined {
                    reason: "synthetic retained cleanup".into(),
                }
            } else {
                SourceCleanup::Confirmed
            },
            stdout_truncated: false,
            stderr_truncated: false,
            artifacts: [
                (
                    "stdout".into(),
                    self.artifacts()
                        .put_bytes("stdout", "text/plain", b"")
                        .unwrap(),
                ),
                (
                    "stderr".into(),
                    self.artifacts()
                        .put_bytes("stderr", "text/plain", b"")
                        .unwrap(),
                ),
                (
                    "diagnostics".into(),
                    self.artifacts()
                        .put_bytes("diagnostics", "application/json", b"[]")
                        .unwrap(),
                ),
            ]
            .into(),
            diagnostics: vec![],
        };
        let record = SourceCheckRecord::new(input.clone()).unwrap();
        input.artifacts.insert(
            "execution-record".into(),
            self.artifacts()
                .put_bytes(
                    "execution-record",
                    "application/vnd.celetixo.source-check-v1",
                    &record.execution_bytes(),
                )
                .unwrap(),
        );
        let mut bytes = BTreeMap::from([
            ("source-patch".to_owned(), patch.canonical_bytes()),
            ("base-snapshot".into(), self.base.canonical_bytes()),
            (
                "result-snapshot".into(),
                application.snapshot().canonical_bytes(),
            ),
            (
                "base-manifest".into(),
                self.base.manifest().canonical_bytes(),
            ),
            (
                "result-manifest".into(),
                application.snapshot().manifest().canonical_bytes(),
            ),
            (
                "application-report".into(),
                application.report().canonical_bytes(),
            ),
            (
                "verification-plan".into(),
                admission.plan().canonical_bytes(),
            ),
            ("execution-admission".into(), admission.canonical_bytes()),
            ("execution-policy".into(), self.policy.canonical_bytes()),
        ]);
        for check in admission.plan().checks() {
            bytes.insert(
                format!("check-definition/{}", check.definition().id()),
                check.definition().canonical_bytes(),
            );
        }
        let provenance = bytes
            .into_iter()
            .map(|(name, bytes)| {
                let object = self
                    .artifacts()
                    .put_bytes(&name, "application/vnd.celetixo.provenance-v1", &bytes)
                    .unwrap();
                (name, object)
            })
            .collect();
        let evidence = SourceEvidence::new(
            SourceEvidenceBinding::for_admission(admission, self.executor.principal().clone()),
            started,
            started,
            vec![SourceCheckRecord::new(input).unwrap()],
            provenance,
            vec![],
        )
        .unwrap();
        let envelope = self
            .artifacts()
            .put_bytes(
                "sealed-evidence",
                SOURCE_EVIDENCE_MEDIA_TYPE,
                &evidence.canonical_bytes(),
            )
            .unwrap();
        (evidence, envelope)
    }
    pub(super) fn new(label: &str) -> Self {
        let selected = scope(label);
        authorize_source(&mut owner(), &selected);
        let authority = Arc::new(super::authority());
        let (engine, admin) = Engine::with_authority_backend(authority.clone());
        let owner = admin
            .grant(
                PrincipalId::new("human").unwrap(),
                selected.clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        let base = Snapshot::new(
            selected.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "synthetic".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "celetixo/reference-source/v1".into(),
                inputs: vec![WorkspaceInput {
                    path: "src/main.rs".into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"old".to_vec()),
                }],
                environment: BTreeMap::new(),
            },
            BTreeMap::new(),
        )
        .unwrap();
        let analysis = RepositoryAnalysis::new(
            &base,
            RepositoryAnalysisInput {
                index_generation: d("index"),
                analyzer_version: "complete-synthetic/v1".into(),
                configuration: d("analysis"),
                elements: [element()].into(),
                dependencies: BTreeMap::new(),
                checks: [(element(), [check_id()].into())].into(),
                gaps: BTreeSet::new(),
            },
        )
        .unwrap();
        engine
            .register_repository(&owner, base.clone(), analysis.clone())
            .unwrap();
        let definition = SourceCheckDefinition::new(SourceCheckInput {
            check: CheckSpec::new(
                check_id(),
                vec!["/tools/synthetic".into()],
                "synthetic/v1".into(),
                d("environment"),
                d("check-config"),
            )
            .unwrap(),
            working_directory: WorkingDirectory::WorkspaceRoot,
            inputs: [(
                "src/main.rs".into(),
                Digest::of("celetixo/source-content/v1", b"new"),
            )]
            .into(),
            outputs: BTreeSet::new(),
            toolchain: ToolchainIdentity {
                executable: "/tools/synthetic".into(),
                executable_digest: d("executable"),
                runtime_closure: d("closure"),
                version: "synthetic/v1".into(),
                platform: "linux".into(),
                architecture: "x86_64".into(),
            },
            dependency_inputs: BTreeMap::new(),
            environment: BTreeMap::new(),
            tier: ExecutionTier::T1,
            backend: backend(),
            resources: resources(),
            network: NetworkPolicy::DenyAll,
            credential_references: BTreeSet::new(),
            expected_artifacts: [
                ArtifactRequirement {
                    name: "stdout".into(),
                    media_type: "text/plain".into(),
                    max_bytes: 4096,
                    required: true,
                },
                ArtifactRequirement {
                    name: "stderr".into(),
                    media_type: "text/plain".into(),
                    max_bytes: 4096,
                    required: true,
                },
                ArtifactRequirement {
                    name: "diagnostics".into(),
                    media_type: "application/json".into(),
                    max_bytes: 4096,
                    required: true,
                },
            ]
            .into(),
            depends_on: BTreeSet::new(),
            failure_policy: FailurePolicy::FailAttempt,
        })
        .unwrap();
        let expiry = now() + 600_000;
        let policy = SourceExecutionPolicy::new(SourcePolicyInput {
            version: 1,
            generation: 1,
            scope: selected.clone(),
            snapshot: base.id(),
            analysis_artifact: analysis.digest(),
            index_generation: analysis.index_generation(),
            capture: SourceCaptureBinding {
                snapshot: base.id(),
                manifest: base.manifest().digest(),
                capture_policy: d("capture"),
                uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
                consistency: SourceCaptureConsistency::ReferenceFixture,
                gaps: BTreeSet::new(),
            },
            patch_policy: PatchPolicy::default(),
            credentials: CredentialPolicy::default(),
            paths: [(
                "src/main.rs".into(),
                SourcePathRule {
                    allowed_operations: [OperationKind::Replace].into(),
                    required_writes: [element()].into(),
                    expected_base: Some(Preimage::of(&base.manifest().inputs[0]).unwrap()),
                },
            )]
            .into(),
            checks: vec![definition],
            invariants: vec![],
            always_required: BTreeSet::new(),
            approved_tiers: [ExecutionTier::T1].into(),
            approved_backends: [backend().digest()].into(),
            resource_ceiling: resources(),
            network_allowed: false,
            approved_credential_references: BTreeSet::new(),
            expires_at_ms: expiry,
            retention_policy: d("retention"),
        })
        .unwrap();
        engine
            .install_source_policy(&owner, &selected, None, policy.clone())
            .unwrap();
        let executor = admin
            .grant_source_executor(
                PrincipalId::new("executor").unwrap(),
                selected.clone(),
                backend(),
                [ExecutionTier::T1].into(),
                resources(),
                expiry,
            )
            .unwrap();
        let artifact_root = PathBuf::from(std::env::var_os("CELETIXO_PG_TEST_CONFIG").unwrap())
            .parent()
            .unwrap()
            .join("source-sql-artifacts");
        std::fs::create_dir_all(&artifact_root).unwrap();
        let artifact_path = artifact_root.join(selected.organization.as_str());
        let artifacts = Arc::new(
            LocalSourceArtifactStore::create(
                &artifact_path,
                selected.clone(),
                d("retention"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        );
        let configuration = Digest::of(
            "source-sql-test-artifact-configuration/v1",
            artifact_path.as_os_str().as_encoded_bytes(),
        );
        let storage = admin
            .register_configured_source_artifact_store(
                selected,
                SourceArtifactAdapter {
                    identity: SourceArtifactAdapterId::new("configured-synthetic-reader").unwrap(),
                    configuration,
                },
                artifacts.clone(),
            )
            .unwrap();
        Self {
            authority,
            engine,
            admin,
            owner,
            executor,
            storage,
            base,
            policy,
            artifacts: Some(artifacts),
            artifact_path,
        }
    }
    pub(super) fn scope(&self) -> &Scope {
        self.base.scope()
    }
    pub(super) fn reopen(&mut self) {
        let before = self.authority.inspect_source(self.scope()).unwrap();
        let authority = Arc::new(super::authority());
        let (engine, admin) = Engine::with_authority_backend(authority.clone());
        let owner = admin
            .authenticate_existing_session(self.scope(), 1, |p, s| {
                assert_eq!(p, self.owner.principal());
                assert_eq!(s, self.scope());
                Ok(())
            })
            .unwrap();
        let executor = admin
            .authenticate_existing_source_executor(self.scope(), 1, |s, g| {
                assert_eq!(s, self.scope());
                assert_eq!(g.backend, backend());
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            engine.current(&self.owner, self.scope()),
            Err(EngineError::Unauthorized)
        ));
        // An orderly cold reopen releases the old embedding's local reader.
        // The local filesystem adapter remains exclusive; no lock is bypassed.
        self.engine = engine;
        self.admin = admin;
        self.owner = owner;
        self.executor = executor;
        drop(self.artifacts.take());
        let artifacts = Arc::new(
            LocalSourceArtifactStore::open(
                &self.artifact_path,
                self.scope().clone(),
                d("retention"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        );
        let configuration = Digest::of(
            "source-sql-test-artifact-configuration/v1",
            self.artifact_path.as_os_str().as_encoded_bytes(),
        );
        let storage = self
            .admin
            .resolve_existing_source_artifact_store(self.scope(), 1, |s, g| {
                assert_eq!(s, self.scope());
                assert_eq!(g.adapter.as_ref().unwrap().configuration, configuration);
                Ok(artifacts.clone())
            })
            .unwrap();
        assert_eq!(authority.inspect_source(self.scope()).unwrap(), before);
        self.authority = authority;
        self.storage = storage;
        self.artifacts = Some(artifacts);
    }
    pub(super) fn propose(&self) {
        self.engine
            .propose(
                &self.owner,
                self.scope(),
                transaction_id(),
                self.base.id(),
                TransactionIntent {
                    objective: "SQL source lifecycle, no command execution".into(),
                    accesses: AccessSet {
                        writes: [element()].into(),
                        ..Default::default()
                    },
                    invariants: vec![],
                    required_checks: vec![self.policy.checks()[&check_id()].input().check.clone()],
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
    }
    pub(super) fn patch(&self, id: &str) -> SourcePatch {
        let context = self
            .engine
            .source_patch_context(&self.owner, self.scope(), &transaction_id())
            .unwrap();
        SourcePatch::new(
            context
                .bind(
                    SourceAttemptId::new(id).unwrap(),
                    "synthetic-sql-fixture".into(),
                    context.maximum_expiry_ms.min(now() + 180_000),
                )
                .unwrap(),
            self.policy.input().patch_policy.clone(),
            vec![PatchOperation::Replace {
                path: "src/main.rs".into(),
                expected: Preimage::of(&self.base.manifest().inputs[0]).unwrap(),
                entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap()
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_evidence_receipts_and_logical_object_associations_reopen_all_assessments() {
    for case in 0..3 {
        let mut h = Harness::new(&format!("source-evidence-{case}"));
        let (patch, admission, application) = h.ready("source-attempt");
        let (evidence, envelope) = h.synthetic_evidence(&patch, &admission, &application, case);
        let expected = match case {
            0 => SourceAttemptState::EvidenceSealed,
            1 => SourceAttemptState::ExecutionFailed,
            _ => SourceAttemptState::EvidenceIncomplete,
        };
        if case == 0 {
            assert_eq!(
                h.engine.record_source_evidence_with_fault(
                    &h.executor,
                    &admission,
                    &h.storage,
                    evidence.clone(),
                    envelope.clone(),
                    SourceEvidenceFault::AfterRetention
                ),
                Err(EngineError::ResponseLost)
            );
        } else {
            h.engine
                .record_source_evidence(
                    &h.executor,
                    &admission,
                    &h.storage,
                    evidence.clone(),
                    envelope.clone(),
                )
                .unwrap();
        }
        h.reopen();
        let (retained, receipt) = h
            .engine
            .source_evidence_record(&h.owner, h.scope(), evidence.id())
            .unwrap();
        assert_eq!(retained, evidence);
        match case {
            0 => assert_eq!(receipt.assessment, SourceEvidenceAssessment::Passed),
            1 => assert!(matches!(
                receipt.assessment,
                SourceEvidenceAssessment::Failed { .. }
            )),
            _ => assert!(matches!(
                receipt.assessment,
                SourceEvidenceAssessment::Incomplete { .. }
            )),
        }
        assert_eq!(
            h.engine
                .record_source_evidence(
                    &h.executor,
                    &admission,
                    &h.storage,
                    evidence.clone(),
                    envelope.clone()
                )
                .unwrap(),
            receipt
        );
        assert_eq!(
            h.engine
                .source_attempt(&h.owner, h.scope(), &admission.binding().source_attempt)
                .unwrap()
                .state,
            expected
        );
        assert_eq!(h.engine.current(&h.owner, h.scope()).unwrap(), h.base);
        let preview = h
            .engine
            .source_publication_candidate(&h.owner, h.scope(), admission.id());
        if case == 0 {
            let candidate = preview.as_ref().unwrap();
            assert_eq!(candidate.application(), &application);
            assert_eq!(candidate.snapshot().revision(), Revision(1));
            assert_eq!(
                candidate.snapshot().manifest(),
                application.snapshot().manifest()
            );
            assert_ne!(candidate.snapshot().id(), admission.resulting_snapshot());
        } else {
            assert_eq!(preview, Err(SourceAttemptError::InvalidState.into()));
        }
        let prepared = if let Ok(candidate) = &preview {
            let analyst = h
                .admin
                .grant(
                    PrincipalId::new("publication-analysis-approver").unwrap(),
                    h.scope().clone(),
                    [Permission::ManageModel].into(),
                )
                .unwrap();
            let analysis = RepositoryAnalysis::new(
                candidate.snapshot(),
                RepositoryAnalysisInput {
                    index_generation: d("publication-index"),
                    analyzer_version: "complete-synthetic-publication/v1".into(),
                    configuration: d("publication-analysis"),
                    elements: [element()].into(),
                    dependencies: BTreeMap::new(),
                    checks: [(element(), [check_id()].into())].into(),
                    gaps: BTreeSet::new(),
                },
            )
            .unwrap();
            let request = h
                .engine
                .prepare_source_commit_request(
                    &h.owner,
                    &analyst,
                    h.scope(),
                    SourceCommitPreparation {
                        admission: admission.id(),
                        analysis,
                        key: IdempotencyKey::new("source-publication-preparation").unwrap(),
                    },
                )
                .unwrap();
            assert_eq!(request.binding, *admission.binding());
            assert_eq!(request.verified_snapshot, application.snapshot().id());
            assert_eq!(request.publication_snapshot, candidate.snapshot().id());
            assert_eq!(h.engine.current(&h.owner, h.scope()).unwrap(), h.base);
            Some(request)
        } else {
            None
        };
        let records = h.authority.inspect_source(h.scope()).unwrap().records;
        assert_eq!(records.evidence.len(), 1);
        assert_eq!(
            records.base.transactions[&transaction_id()].state,
            if case == 0 {
                TransactionState::Validating
            } else {
                TransactionState::Executing
            }
        );
        let a = snapshots_args(h.scope());
        let row=owner().query_one("SELECT count(*),count(DISTINCT object_digest) FROM celetixo.artifact_references WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND reference_name IN ('stdout','stderr')",&[&a[0],&a[1],&a[2]]).unwrap();
        assert_eq!(row.get::<_, i64>(0), 2);
        assert_eq!(row.get::<_, i64>(1), 1);
        h.admin.revoke_source_artifact_store(&h.storage).unwrap();
        let fresh_authority = Arc::new(super::authority());
        let (fresh, fresh_admin) = Engine::with_authority_backend(fresh_authority.clone());
        let session = fresh_admin
            .authenticate_existing_session(h.scope(), 1, |p, s| {
                assert_eq!(p, h.owner.principal());
                assert_eq!(s, h.scope());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            fresh
                .source_evidence_record(&session, h.scope(), evidence.id())
                .unwrap(),
            (evidence, receipt)
        );
        // Read-only publication analysis works after a cold Engine open, with
        // no restored runtime executor/reader and a revoked artifact store.
        // This cannot authorize commit or attest current object availability.
        assert_eq!(
            fresh.source_publication_candidate(&session, h.scope(), admission.id()),
            preview
        );
        assert_eq!(fresh.current(&session, h.scope()).unwrap(), h.base);
        if let Some(request) = prepared {
            let approver = fresh_admin
                .authenticate_existing_session(h.scope(), request.analysis_grant, |p, s| {
                    assert_eq!(p, &request.analysis_principal);
                    assert_eq!(s, h.scope());
                    Ok(())
                })
                .unwrap();
            let restored = celetixo_engine::persistence::restore_source_commit_request(
                h.scope(),
                session.principal(),
                request.key.clone(),
                request.digest(h.scope(), session.principal()),
                &request.canonical_bytes(h.scope(), session.principal()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                fresh
                    .prepare_source_commit_request(
                        &session,
                        &approver,
                        h.scope(),
                        SourceCommitPreparation {
                            admission: request.admission,
                            analysis: restored.analysis.clone(),
                            key: request.key.clone(),
                        }
                    )
                    .unwrap(),
                restored
            );
            assert_eq!(fresh.current(&session, h.scope()).unwrap(), h.base);
        }
        assert_eq!(
            fresh_authority
                .inspect_source(h.scope())
                .unwrap()
                .records
                .evidence,
            records.evidence
        );
    }
}
fn snapshots_args(scope: &Scope) -> [&str; 3] {
    [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ]
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_admission_publication_faults_withhold_results_and_recover_original_rejection() {
    let h = Harness::new("source-sql-faults");
    h.prepare();
    let patch = h.patch("admitted-after-retry");
    let key = IdempotencyKey::new("source-operation").unwrap();
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let injected = Arc::new(super::authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::BeforeCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(injected);
    let session = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.admit_source_attempt(&session, h.scope(), patch.clone(), key.clone()),
        Err(EngineError::Authority(AuthorityFailure::Unavailable))
    );
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
    let injected = Arc::new(super::authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::AfterCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(injected);
    let session = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.admit_source_attempt(&session, h.scope(), patch.clone(), key.clone()),
        Err(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    );
    let published = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(published.records.attempts.len(), 1);
    assert_eq!(published.records.operations.len(), 1);
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), patch.clone(), key)
        .unwrap();
    assert_eq!(
        published.records.attempts[&admission.binding().source_attempt].admission,
        admission
    );
    let mut binding = patch.binding().clone();
    binding.source_attempt = SourceAttemptId::new("rejected-after-lost-reply").unwrap();
    binding.transaction_attempt = 999;
    let invalid =
        SourcePatch::new(binding, patch.policy().clone(), patch.operations().to_vec()).unwrap();
    let key = IdempotencyKey::new("rejected-operation").unwrap();
    assert_eq!(
        engine.admit_source_attempt(&session, h.scope(), invalid.clone(), key.clone()),
        Err(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    );
    let persisted = h.authority.inspect_source(h.scope()).unwrap();
    let expected = persisted.records.operations[&key]
        .outcome
        .clone()
        .unwrap_err();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, h.scope(), invalid, key),
        Err(expected)
    );
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(after.records.operations, persisted.records.operations);
    assert_eq!(after.records.rejections, persisted.records.rejections);
    assert_eq!(after.records.attempts.len(), 1);
    assert_eq!(after.records.rejections.len(), 1);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_final_database_clock_rejects_an_execution_transition_after_lease_expiry() {
    let h = Harness::new("source-final-clock");
    let (_, admission, _) = h.ready_with_ttl("source-attempt", 9_000);
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let expires = before.records.base.transactions[&transaction_id()]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let injected = Arc::new(super::authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            assert!(now() < expires, "the transition must load a live lease");
            let remaining = expires.saturating_add(2).saturating_sub(now());
            assert!(remaining <= 9_002, "expiry checkpoint must stay bounded");
            std::thread::sleep(std::time::Duration::from_millis(remaining));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(injected);
    let executor = admin
        .authenticate_existing_source_executor(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.finish_source_attempt(
            &executor,
            &admission,
            SourceAttemptState::ExecutionFailed,
            "late result".into()
        ),
        Err(EngineError::Authority(AuthorityFailure::Contention))
    );
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_original_rejections_collisions_and_historical_policy_survive_head_replacement() {
    let mut h = Harness::new("source-original-requests");
    let (patch, admission, _) = h.ready("source-attempt");
    let mut binding = patch.binding().clone();
    binding.source_attempt = SourceAttemptId::new("invalid-attempt-999").unwrap();
    binding.transaction_attempt = 999;
    let rejected =
        SourcePatch::new(binding, patch.policy().clone(), patch.operations().to_vec()).unwrap();
    let key = IdempotencyKey::new("original-rejection").unwrap();
    let original = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), rejected.clone(), key.clone())
        .unwrap_err();
    let collision = IdempotencyKey::new("collision").unwrap();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, h.scope(), patch.clone(), collision.clone()),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AttemptIdentity
        ))
    );
    h.reopen();
    let original_records = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(original_records.operations[&key].target_attempt, 0);
    assert_eq!(
        original_records.operations[&key]
            .request
            .patch()
            .binding()
            .transaction_attempt,
        999
    );
    assert_eq!(original_records.rejections.len(), 1);
    let mut input = h.policy.input().clone();
    input.generation = 2;
    let replacement = SourceExecutionPolicy::new(input).unwrap();
    h.engine
        .install_source_policy(
            &h.owner,
            h.scope(),
            Some(h.policy.digest()),
            replacement.clone(),
        )
        .unwrap();
    h.reopen();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, h.scope(), rejected, key.clone()),
        Err(original)
    );
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, h.scope(), patch, collision),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AttemptIdentity
        ))
    );
    let records = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(records.policy_head, Some(replacement.digest()));
    assert_eq!(records.policies.len(), 2);
    assert_eq!(records.operations, original_records.operations);
    assert_eq!(records.rejections, original_records.rejections);
    assert_eq!(
        records.attempts[&admission.binding().source_attempt]
            .admission
            .policy(),
        h.policy.digest()
    );
    assert_eq!(
        h.engine
            .source_attempt(&h.owner, h.scope(), &admission.binding().source_attempt)
            .unwrap()
            .state,
        SourceAttemptState::RevalidationRequired
    );
    h.reopen();
    assert_eq!(h.engine.current(&h.owner, h.scope()).unwrap(), h.base);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_lifecycle_admission_claim_and_failure_reopen_each_phase_without_native_patches() {
    let mut h = Harness::new("source-lifecycle");
    h.reopen();
    h.propose();
    h.reopen();
    h.engine
        .analyze(&h.owner, h.scope(), &transaction_id())
        .unwrap();
    h.reopen();
    h.engine
        .reserve(&h.owner, h.scope(), &transaction_id(), 300_000)
        .unwrap();
    h.reopen();
    h.engine
        .execute(&h.owner, h.scope(), &transaction_id())
        .unwrap();
    h.reopen();
    let patch = h.patch("source-attempt");
    let key = IdempotencyKey::new("admission").unwrap();
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), patch.clone(), key.clone())
        .unwrap();
    h.reopen();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, h.scope(), patch, key)
            .unwrap(),
        admission
    );
    h.engine
        .claim_source_attempt(&h.executor, &admission)
        .unwrap();
    h.reopen();
    assert!(matches!(
        h.engine.claim_source_attempt(&h.executor, &admission),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AlreadyClaimed
        ))
    ));
    let application = h
        .engine
        .source_application(&h.executor, &admission)
        .unwrap();
    assert_eq!(application.snapshot().id(), admission.resulting_snapshot());
    h.engine
        .mark_source_patch_applied(&h.executor, &admission)
        .unwrap();
    h.reopen();
    h.engine
        .begin_source_verification(&h.executor, &admission)
        .unwrap();
    h.reopen();
    h.engine
        .finish_source_attempt(
            &h.executor,
            &admission,
            SourceAttemptState::ExecutionFailed,
            "injected infrastructure failure".into(),
        )
        .unwrap();
    h.reopen();
    let view = h
        .engine
        .source_attempt(&h.owner, h.scope(), &admission.binding().source_attempt)
        .unwrap();
    assert_eq!(view.state, SourceAttemptState::ExecutionFailed);
    assert_eq!(h.engine.current(&h.owner, h.scope()).unwrap(), h.base);
    let rows = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(
        rows.base.transactions[&transaction_id()].state,
        TransactionState::Executing
    );
    let attempt = &rows.base.transactions[&transaction_id()].attempts[0];
    assert!(attempt.patch.is_none() && attempt.candidate.is_none() && attempt.evidence.is_empty());
    assert_eq!(rows.operations.len(), 1);
    assert_eq!(rows.attempts.len(), 1);
}

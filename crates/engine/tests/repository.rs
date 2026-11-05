//! Engine-owned source-analysis admission and authority boundaries.
//! Complete artifacts here describe a deliberately finite test model. They do
//! not claim that a production parser or language server proves full coverage.
use celetixo_engine::{
    access::{AccessSet, Completeness, Condition, Invariant, Predicate},
    engine::{Administrator, Engine, EngineError, InMemoryStore, Permission, Session},
    evidence::CheckSpec,
    ids::*,
    model::EngineeringModel,
    patch::{Change, Patch},
    repository::{
        AnalysisGap, RepositoryAnalysis, RepositoryAnalysisError, RepositoryAnalysisInput,
    },
    snapshot::{
        FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest,
        WorkspaceInput,
    },
    transaction::{TransactionIntent, TransactionState},
};
use std::collections::{BTreeMap, BTreeSet};

fn element(name: &str) -> ElementId {
    ElementId::new(name).unwrap()
}
fn digest(value: &str) -> Digest {
    Digest::of("repository-test/v1", value.as_bytes())
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repo").unwrap(),
    }
}
fn source_snapshot(scope: Scope, text: &str) -> Snapshot {
    Snapshot::new(
        scope,
        Revision(0),
        SnapshotManifest {
            base_commit: "git:unchanged-head".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "celetixo/git-capture/v1;fixture".into(),
            inputs: vec![WorkspaceInput {
                path: "main.rs".into(),
                layer: InputLayer::WorkingTree,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: Some(text.as_bytes().to_vec()),
            }],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn reference_snapshot() -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "reference-base".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "declared-integer-reference/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        [(element("root"), 1)].into(),
    )
    .unwrap()
}
fn input(generation: &str, incomplete: bool) -> RepositoryAnalysisInput {
    RepositoryAnalysisInput {
        index_generation: digest(generation),
        analyzer_version: "trusted-analyzer/1".into(),
        configuration: digest("configuration"),
        elements: [
            element("root"),
            element("consumer"),
            element("test"),
            element("isolated"),
        ]
        .into(),
        dependencies: [
            (element("consumer"), [element("root")].into()),
            (element("test"), [element("consumer")].into()),
        ]
        .into(),
        checks: [(
            element("test"),
            [CheckId::new("test:consumer").unwrap()].into(),
        )]
        .into(),
        gaps: if incomplete {
            [AnalysisGap {
                stage: "semantic".into(),
                path: Some("main.rs".into()),
                detail: "unresolved external reference".into(),
            }]
            .into()
        } else {
            BTreeSet::new()
        },
    }
}
fn artifact(snapshot: &Snapshot, generation: &str, incomplete: bool) -> RepositoryAnalysis {
    RepositoryAnalysis::new(snapshot, input(generation, incomplete)).unwrap()
}
fn access() -> AccessSet {
    AccessSet {
        writes: [element("root")].into(),
        ..Default::default()
    }
}
fn model() -> EngineeringModel {
    EngineeringModel {
        generation: 10,
        analyzer: "reference".into(),
        version: "1".into(),
        configuration: digest("reference-model"),
        dependencies: BTreeMap::new(),
        completeness: Completeness::Complete,
    }
}
fn intent(accesses: AccessSet) -> TransactionIntent {
    TransactionIntent {
        objective: "bounded source impact proposal".into(),
        accesses,
        invariants: vec![],
        required_checks: vec![
            CheckSpec::new(
                CheckId::new("test:consumer").unwrap(),
                vec!["trusted-test".into()],
                "fixture-toolchain".into(),
                digest("environment"),
                digest("check-configuration"),
            )
            .unwrap(),
        ],
        depends_on: BTreeSet::new(),
    }
}
struct Harness {
    admin: Administrator,
    engine: Engine,
    operator: Session,
    snapshot: Snapshot,
    analysis: RepositoryAnalysis,
}
impl Harness {
    fn new(incomplete: bool) -> Self {
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let operator = admin
            .grant(
                PrincipalId::new("operator").unwrap(),
                scope(),
                Permission::ALL.into(),
            )
            .unwrap();
        let snapshot = source_snapshot(scope(), "fn root() {}\n");
        let analysis = artifact(&snapshot, "index-1", incomplete);
        engine
            .register_repository(&operator, snapshot.clone(), analysis.clone())
            .unwrap();
        Self {
            admin,
            engine,
            operator,
            snapshot,
            analysis,
        }
    }
    fn grant(&self, name: &str, permissions: BTreeSet<Permission>) -> Session {
        self.admin
            .grant(PrincipalId::new(name).unwrap(), scope(), permissions)
            .unwrap()
    }
    fn inspect(
        &self,
        index: Digest,
        accesses: &AccessSet,
    ) -> Result<celetixo_engine::repository::RepositoryImpact, EngineError> {
        self.engine.inspect_repository_impact(
            &self.operator,
            &scope(),
            self.snapshot.id(),
            index,
            accesses,
        )
    }
    fn propose(&self, name: &str, request: TransactionIntent) -> TransactionId {
        let id = TransactionId::new(name).unwrap();
        self.engine
            .propose(
                &self.operator,
                &scope(),
                id.clone(),
                self.snapshot.id(),
                request,
            )
            .unwrap();
        id
    }
}

#[test]
fn contracts_bind_exact_snapshot_scope_index_configuration_facts_and_coverage() {
    let base = source_snapshot(scope(), "fn root() {}\n");
    let original = artifact(&base, "index-1", false);
    let mut variants = Vec::new();
    let mut changed = input("index-2", false);
    variants.push(changed.clone());
    changed = input("index-1", false);
    changed.analyzer_version = "trusted-analyzer/2".into();
    variants.push(changed);
    let mut changed = input("index-1", false);
    changed.configuration = digest("other-config");
    variants.push(changed);
    let mut changed = input("index-1", false);
    changed.elements.insert(element("new"));
    variants.push(changed);
    let mut changed = input("index-1", false);
    changed
        .dependencies
        .insert(element("isolated"), [element("root")].into());
    variants.push(changed);
    let mut changed = input("index-1", false);
    changed
        .checks
        .insert(element("root"), [CheckId::new("test:root").unwrap()].into());
    variants.push(changed);
    variants.push(input("index-1", true));
    for variant in variants {
        assert_ne!(
            RepositoryAnalysis::new(&base, variant).unwrap().digest(),
            original.digest()
        );
    }
    assert_eq!(original, artifact(&base, "index-1", false));
    assert_eq!(original.analyzer_version(), "trusted-analyzer/1");
    assert_eq!(original.configuration(), digest("configuration"));
    assert!(original.is_complete());
    let dirty = source_snapshot(scope(), "fn root() { changed(); }\n");
    assert_eq!(base.manifest().base_commit, dirty.manifest().base_commit);
    assert_eq!(
        original.validate(&dirty, original.index_generation()),
        Err(RepositoryAnalysisError::SnapshotMismatch)
    );
    assert_eq!(
        original.validate(&base, digest("stale-index")),
        Err(RepositoryAnalysisError::StaleGeneration)
    );
    let mut foreign = scope();
    foreign.workspace = WorkspaceId::new("other-workspace").unwrap();
    let foreign = source_snapshot(foreign, "fn root() {}\n");
    assert_eq!(
        original.validate(&foreign, original.index_generation()),
        Err(RepositoryAnalysisError::SnapshotMismatch)
    );
    assert_ne!(
        artifact(&foreign, "index-1", false).digest(),
        original.digest()
    );
}
#[test]
fn constructor_rejects_dangling_dependencies_checks_and_missing_provenance() {
    let snapshot = source_snapshot(scope(), "fn root() {}\n");
    for variant in 0..3 {
        let mut invalid = input("index-1", false);
        match variant {
            0 => {
                invalid
                    .dependencies
                    .insert(element("missing"), [element("root")].into());
            }
            1 => {
                invalid
                    .dependencies
                    .insert(element("root"), [element("missing")].into());
            }
            _ => {
                invalid
                    .checks
                    .insert(element("missing"), [CheckId::new("check").unwrap()].into());
            }
        }
        assert_eq!(
            RepositoryAnalysis::new(&snapshot, invalid),
            Err(RepositoryAnalysisError::UnknownElement(element("missing")))
        );
    }
    let mut invalid = input("index-1", false);
    invalid.analyzer_version = "  ".into();
    assert_eq!(
        RepositoryAnalysis::new(&snapshot, invalid),
        Err(RepositoryAnalysisError::InvalidProvenance)
    );
    let mut invalid = input("index-1", true);
    invalid.gaps = BTreeSet::from([AnalysisGap {
        stage: "".into(),
        path: None,
        detail: "unresolved".into(),
    }]);
    assert_eq!(
        RepositoryAnalysis::new(&snapshot, invalid),
        Err(RepositoryAnalysisError::InvalidProvenance)
    );
}
#[test]
fn engine_queries_transitive_impact_checks_and_exact_registered_artifact() {
    let h = Harness::new(false);
    let result = h.inspect(h.analysis.index_generation(), &access()).unwrap();
    assert_eq!(result.scope, scope());
    assert_eq!(result.snapshot, h.snapshot.id());
    assert_eq!(result.index_generation, h.analysis.index_generation());
    assert_eq!(result.artifact, h.analysis.digest());
    assert!(result.analysis.is_complete());
    assert_eq!(result.analysis.stamp().configuration, h.analysis.digest());
    assert!(
        result
            .analysis
            .derived()
            .reads
            .contains(&element("consumer"))
    );
    assert!(result.analysis.derived().reads.contains(&element("test")));
    assert_eq!(
        result.justified_checks,
        [CheckId::new("test:consumer").unwrap()].into()
    );
    assert_eq!(h.engine.current(&h.operator, &scope()).unwrap(), h.snapshot);
    assert_eq!(
        h.inspect(digest("stale"), &access()),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::StaleGeneration
        ))
    );
    assert_eq!(
        h.engine.inspect_repository_impact(
            &h.operator,
            &scope(),
            source_snapshot(scope(), "different").id(),
            h.analysis.index_generation(),
            &access()
        ),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::SnapshotMismatch
        ))
    );
}
#[test]
fn source_registration_requires_both_scoped_registration_and_model_authority() {
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let snapshot = source_snapshot(scope(), "fn root() {}\n");
    let analysis = artifact(&snapshot, "index-1", false);
    for (name, permissions) in [
        ("register-only", [Permission::Register].into()),
        ("model-only", [Permission::ManageModel].into()),
    ] {
        let actor = admin
            .grant(PrincipalId::new(name).unwrap(), scope(), permissions)
            .unwrap();
        assert_eq!(
            engine.register_repository(&actor, snapshot.clone(), analysis.clone()),
            Err(EngineError::Unauthorized)
        );
    }
    let operator = admin
        .grant(
            PrincipalId::new("operator").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    let dirty = source_snapshot(scope(), "different");
    assert_eq!(
        engine.register_repository(&operator, dirty, analysis.clone()),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::SnapshotMismatch
        ))
    );
    assert_eq!(
        engine.current(&operator, &scope()),
        Err(EngineError::NotFound)
    );
    engine
        .register_repository(&operator, snapshot.clone(), analysis.clone())
        .unwrap();
    assert_eq!(
        engine.register_repository(&operator, snapshot, analysis),
        Err(EngineError::AlreadyExists)
    );
}
#[test]
fn analysis_operations_enforce_organization_workspace_repository_store_and_revocation_scope() {
    let h = Harness::new(false);
    for dimension in 0..3 {
        let mut wrong = scope();
        match dimension {
            0 => wrong.organization = OrganizationId::new("foreign").unwrap(),
            1 => wrong.workspace = WorkspaceId::new("foreign").unwrap(),
            _ => wrong.repository = RepositoryId::new("foreign").unwrap(),
        };
        let actor = h
            .admin
            .grant(
                PrincipalId::new(format!("wrong-{dimension}")).unwrap(),
                wrong,
                Permission::ALL.into(),
            )
            .unwrap();
        assert_eq!(
            h.engine.inspect_repository_impact(
                &actor,
                &scope(),
                h.snapshot.id(),
                h.analysis.index_generation(),
                &access()
            ),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            h.engine.install_repository_analysis(
                &actor,
                &scope(),
                h.analysis.digest(),
                h.analysis.clone()
            ),
            Err(EngineError::Unauthorized)
        );
    }
    let no_analysis = h.grant("reader", [Permission::Read].into());
    assert_eq!(
        h.engine.inspect_repository_impact(
            &no_analysis,
            &scope(),
            h.snapshot.id(),
            h.analysis.index_generation(),
            &access()
        ),
        Err(EngineError::Unauthorized)
    );
    let analyst = h.grant("analyst", [Permission::Analyze].into());
    h.engine
        .inspect_repository_impact(
            &analyst,
            &scope(),
            h.snapshot.id(),
            h.analysis.index_generation(),
            &access(),
        )
        .unwrap();
    assert_eq!(
        h.engine.install_repository_analysis(
            &analyst,
            &scope(),
            h.analysis.digest(),
            h.analysis.clone()
        ),
        Err(EngineError::Unauthorized)
    );
    h.admin.revoke(&analyst).unwrap();
    assert_eq!(
        h.engine.inspect_repository_impact(
            &analyst,
            &scope(),
            h.snapshot.id(),
            h.analysis.index_generation(),
            &access()
        ),
        Err(EngineError::Unauthorized)
    );
    let (_, foreign_admin) = InMemoryStore::new();
    let foreign = foreign_admin
        .grant(
            PrincipalId::new("foreign-store").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    assert_eq!(
        h.engine.inspect_repository_impact(
            &foreign,
            &scope(),
            h.snapshot.id(),
            h.analysis.index_generation(),
            &access()
        ),
        Err(EngineError::Unauthorized)
    );
}
#[test]
fn incomplete_source_analysis_is_queryable_but_cannot_admit_or_commit() {
    let h = Harness::new(true);
    let result = h.inspect(h.analysis.index_generation(), &access()).unwrap();
    assert!(!result.analysis.is_complete());
    assert!(
        matches!(result.analysis.completeness(),Completeness::Incomplete{reasons} if reasons.iter().any(|reason|reason.contains("unresolved external")))
    );
    let id = h.propose("incomplete", intent(access()));
    assert_eq!(
        h.engine.analyze(&h.operator, &scope(), &id),
        Err(EngineError::IncompleteAnalysis)
    );
    assert_eq!(
        h.engine
            .transaction(&h.operator, &scope(), &id)
            .unwrap()
            .state,
        TransactionState::Proposed
    );
    assert_eq!(
        h.engine.attempts(&h.operator, &scope(), &id).unwrap()[0].analysis,
        None
    );
    assert_eq!(
        h.engine.commit(
            &h.operator,
            &scope(),
            &id,
            IdempotencyKey::new("no-commit").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(h.engine.current(&h.operator, &scope()).unwrap(), h.snapshot);
}
#[test]
fn index_install_uses_artifact_compare_and_swap_and_preserves_revision() {
    let h = Harness::new(false);
    let next = artifact(&h.snapshot, "index-2", false);
    assert_eq!(
        h.engine.install_repository_analysis(
            &h.operator,
            &scope(),
            digest("wrong-expected"),
            next.clone()
        ),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::StaleGeneration
        ))
    );
    h.inspect(h.analysis.index_generation(), &access()).unwrap();
    h.engine
        .install_repository_analysis(&h.operator, &scope(), h.analysis.digest(), next.clone())
        .unwrap();
    let result = h.inspect(next.index_generation(), &access()).unwrap();
    assert_eq!(result.artifact, next.digest());
    assert_eq!(result.analysis.stamp().index_generation, 2);
    // Retrying an acknowledged/lost-response installation is idempotent.
    h.engine
        .install_repository_analysis(&h.operator, &scope(), h.analysis.digest(), next.clone())
        .unwrap();
    assert_eq!(
        h.inspect(next.index_generation(), &access())
            .unwrap()
            .analysis
            .stamp()
            .index_generation,
        2
    );
    assert_eq!(
        h.engine.install_repository_analysis(
            &h.operator,
            &scope(),
            h.analysis.digest(),
            h.analysis.clone()
        ),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::StaleGeneration
        ))
    );
    assert_eq!(
        h.inspect(h.analysis.index_generation(), &access()),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::StaleGeneration
        ))
    );
    let other_snapshot = source_snapshot(scope(), "dirty newer source");
    assert_eq!(
        h.engine.install_repository_analysis(
            &h.operator,
            &scope(),
            next.digest(),
            artifact(&other_snapshot, "index-3", false)
        ),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::SnapshotMismatch
        ))
    );
    assert_eq!(h.engine.current(&h.operator, &scope()).unwrap(), h.snapshot);
    assert_eq!(
        h.inspect(next.index_generation(), &access())
            .unwrap()
            .artifact,
        next.digest()
    );
}
#[test]
fn changed_analyzer_coverage_invalidates_old_transactions_and_new_admission() {
    let h = Harness::new(false);
    let old = h.propose("old", intent(access()));
    let next = artifact(&h.snapshot, "index-2", true);
    h.engine
        .install_repository_analysis(&h.operator, &scope(), h.analysis.digest(), next.clone())
        .unwrap();
    assert_eq!(
        h.engine.analyze(&h.operator, &scope(), &old),
        Err(EngineError::ModelChanged)
    );
    let fresh = h.propose("new", intent(access()));
    assert_eq!(
        h.engine.analyze(&h.operator, &scope(), &fresh),
        Err(EngineError::IncompleteAnalysis)
    );
    assert!(
        !h.inspect(next.index_generation(), &access())
            .unwrap()
            .analysis
            .is_complete()
    );
}
#[test]
fn source_analysis_rejects_unknown_explicit_and_invariant_identities() {
    let h = Harness::new(false);
    for accesses in [
        AccessSet {
            reads: [element("missing")].into(),
            ..Default::default()
        },
        AccessSet {
            writes: [element("missing")].into(),
            ..Default::default()
        },
        AccessSet {
            invariant_scopes: [element("missing")].into(),
            ..Default::default()
        },
    ] {
        assert_eq!(
            h.inspect(h.analysis.index_generation(), &accesses),
            Err(EngineError::RepositoryAnalysis(
                RepositoryAnalysisError::UnknownElement(element("missing"))
            ))
        );
    }
    let id = h.propose(
        "unknown-access",
        intent(AccessSet {
            writes: [element("missing")].into(),
            ..Default::default()
        }),
    );
    assert_eq!(
        h.engine.analyze(&h.operator, &scope(), &id),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::UnknownElement(element("missing"))
        ))
    );
    let mut request = intent(access());
    request.invariants.push(Invariant {
        id: InvariantId::new("unknown-invariant").unwrap(),
        condition: Condition::Present(element("missing")),
    });
    let id = h.propose("unknown-invariant", request);
    assert_eq!(
        h.engine.analyze(&h.operator, &scope(), &id),
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::UnknownElement(element("missing"))
        ))
    );
}
#[test]
fn predicate_membership_covers_isolated_entities_during_query_and_admission() {
    let h = Harness::new(false);
    let accesses = AccessSet {
        predicates: [Predicate::Prefix("isolated".into())].into(),
        ..Default::default()
    };
    let result = h.inspect(h.analysis.index_generation(), &accesses).unwrap();
    assert!(
        result
            .analysis
            .derived()
            .reads
            .contains(&element("isolated"))
    );
    let id = h.propose("predicate", intent(accesses));
    assert!(
        h.engine
            .analyze(&h.operator, &scope(), &id)
            .unwrap()
            .reads
            .contains(&element("isolated"))
    );
}
#[test]
fn integer_model_patch_and_revalidation_paths_cannot_mutate_source_repositories() {
    let h = Harness::new(false);
    assert_eq!(
        h.engine.update_model(&h.operator, &scope(), model()),
        Err(EngineError::SourceAnalysisRequired)
    );
    let id = h.propose("integer-bypass", intent(access()));
    h.engine.analyze(&h.operator, &scope(), &id).unwrap();
    h.engine.reserve(&h.operator, &scope(), &id, 100).unwrap();
    h.engine.execute(&h.operator, &scope(), &id).unwrap();
    let patch = Patch::new(
        h.snapshot.id(),
        [(
            element("root"),
            Change {
                expected: None,
                value: Some(7),
            },
        )]
        .into(),
    )
    .unwrap();
    assert_eq!(
        h.engine.submit_patch(&h.operator, &scope(), &id, patch),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(
        h.engine.revalidate(&h.operator, &scope(), &id),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(
        h.engine.commit(
            &h.operator,
            &scope(),
            &id,
            IdempotencyKey::new("integer-commit").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(
        h.engine
            .transaction(&h.operator, &scope(), &id)
            .unwrap()
            .state,
        TransactionState::Executing
    );
    assert_eq!(h.engine.current(&h.operator, &scope()).unwrap(), h.snapshot);
}
#[test]
fn legacy_registration_cannot_disguise_any_captured_input_layer_as_integer_state() {
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let operator = admin
        .grant(
            PrincipalId::new("operator").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    let reference = reference_snapshot();
    for layer in InputLayer::ALL {
        let mut manifest = reference.manifest().clone();
        manifest.inputs.push(WorkspaceInput {
            path: "input".into(),
            layer,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(vec![]),
        });
        let captured =
            Snapshot::new(scope(), Revision(0), manifest, reference.state().clone()).unwrap();
        assert_eq!(
            engine.register(&operator, captured, model()),
            Err(EngineError::SourceAnalysisRequired)
        );
    }
    for variant in 0..3 {
        let mut manifest = reference.manifest().clone();
        match variant {
            0 => manifest.provenance = "celetixo/git-capture/v1;empty-repo".into(),
            1 => {
                manifest
                    .environment
                    .insert("celetixo.capture.policy".into(), "bound-policy".into());
            }
            _ => {
                manifest
                    .inclusion_policy
                    .excluded_paths
                    .insert("missing-input".into(), "declared exclusion".into());
            }
        }
        let empty_capture =
            Snapshot::new(scope(), Revision(0), manifest, reference.state().clone()).unwrap();
        assert_eq!(
            engine.register(&operator, empty_capture, model()),
            Err(EngineError::SourceAnalysisRequired)
        );
    }
    engine
        .register(&operator, reference.clone(), model())
        .unwrap();
    assert_eq!(engine.current(&operator, &scope()).unwrap(), reference);
    assert_eq!(
        engine.install_repository_analysis(
            &operator,
            &scope(),
            digest("unknown"),
            artifact(&reference, "index", false)
        ),
        Err(EngineError::SourceAnalysisRequired)
    );
}

#[test]
fn concurrent_analysis_replacements_have_one_atomic_compare_and_swap_winner() {
    use std::sync::{Arc, Barrier};
    let h = Harness::new(false);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for version in ["race-a", "race-b"] {
        let engine = h.engine.clone();
        let actor = h.operator.clone();
        let next = artifact(&h.snapshot, version, false);
        let expected = h.analysis.digest();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let outcome =
                engine.install_repository_analysis(&actor, &scope(), expected, next.clone());
            (next, outcome)
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| *result
                == Err(EngineError::RepositoryAnalysis(
                    RepositoryAnalysisError::StaleGeneration
                )))
            .count(),
        1
    );
    let winner = &results
        .iter()
        .find(|(_, outcome)| outcome.is_ok())
        .unwrap()
        .0;
    let installed = h.inspect(winner.index_generation(), &access()).unwrap();
    assert_eq!(installed.artifact, winner.digest());
    assert_eq!(installed.analysis.stamp().index_generation, 2);
    assert_eq!(h.engine.current(&h.operator, &scope()).unwrap(), h.snapshot);
}

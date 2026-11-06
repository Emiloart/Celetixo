use celetixo_engine::ids::{
    Digest, OrganizationId, PrincipalId, RepositoryId, Revision, Scope, SourceAttemptId,
    TransactionId, WorkspaceId,
};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, State,
    WorkspaceInput, effective_workspace,
};
use celetixo_engine::source_patch::{
    DecodeLimits, PatchBinding, PatchEntry, PatchOperation, PatchPolicy, Payload, Preimage,
    SourcePatch, SourcePatchError, SymlinkPolicy,
};
use std::collections::BTreeMap;

fn digest(text: &str) -> Digest {
    Digest::of("test/source-patch", text.as_bytes())
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("ws").unwrap(),
        repository: RepositoryId::new("repo").unwrap(),
    }
}
fn input(path: &str, bytes: &[u8], layer: InputLayer) -> WorkspaceInput {
    WorkspaceInput {
        path: path.into(),
        layer,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(bytes.into()),
    }
}
fn snapshot(inputs: Vec<WorkspaceInput>) -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(7),
        SnapshotManifest {
            base_commit: "0123456789012345678901234567890123456789".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "test captured source".into(),
            inputs,
            environment: BTreeMap::new(),
        },
        State::new(),
    )
    .unwrap()
}
fn base() -> Snapshot {
    snapshot(vec![
        input("a", b"alpha", InputLayer::Base),
        input("b", b"beta", InputLayer::Base),
        input("c", b"gamma", InputLayer::Base),
    ])
}
fn binding(base: &Snapshot) -> PatchBinding {
    PatchBinding {
        scope: base.scope().clone(),
        principal: PrincipalId::new("human").unwrap(),
        transaction: TransactionId::new("tx").unwrap(),
        transaction_attempt: 1,
        source_attempt: SourceAttemptId::new("attempt").unwrap(),
        base_snapshot: base.id(),
        base_revision: base.revision(),
        analysis_artifact: digest("analysis"),
        index_generation: digest("index"),
        impact_request: digest("request"),
        impact_result: digest("result"),
        created_at_ms: 100,
        expires_at_ms: 1000,
        producer: "controlled fixture v1".into(),
    }
}
fn pre(base: &Snapshot, path: &str) -> Preimage {
    Preimage::of(effective_workspace(base).unwrap().get(path).unwrap()).unwrap()
}
fn file(bytes: &[u8]) -> PatchEntry {
    PatchEntry::file(bytes.into(), FileMode::Regular).unwrap()
}
fn replace(base: &Snapshot, path: &str, bytes: &[u8]) -> PatchOperation {
    PatchOperation::Replace {
        path: path.into(),
        expected: pre(base, path),
        entry: file(bytes),
    }
}
fn patch(base: &Snapshot, operations: Vec<PatchOperation>) -> SourcePatch {
    SourcePatch::new(binding(base), PatchPolicy::default(), operations).unwrap()
}
fn content(snapshot: &Snapshot, path: &str) -> Option<Vec<u8>> {
    effective_workspace(snapshot)
        .unwrap()
        .get(path)
        .and_then(|i| i.bytes.clone())
}

#[test]
fn binary_operations_preserve_source_and_revision_and_replay_exactly() {
    let base = base();
    let original = base.clone();
    let patch = patch(
        &base,
        vec![
            PatchOperation::Create {
                path: "nested/new.bin".into(),
                entry: file(&[0, 255, 13, 10]),
            },
            replace(&base, "a", b"// comments\nnew bytes\n"),
            PatchOperation::Delete {
                path: "b".into(),
                expected: pre(&base, "b"),
            },
            PatchOperation::SetMode {
                path: "c".into(),
                expected: pre(&base, "c"),
                mode: FileMode::Executable,
            },
        ],
    );
    let applied = patch.apply(&base).unwrap();
    applied.validate(&base, &patch).unwrap();
    assert_eq!(base, original);
    assert_eq!(applied.snapshot().revision(), Revision(7));
    assert_ne!(applied.snapshot().id(), base.id());
    assert_eq!(
        content(applied.snapshot(), "nested/new.bin"),
        Some(vec![0, 255, 13, 10])
    );
    assert_eq!(
        content(applied.snapshot(), "a"),
        Some(b"// comments\nnew bytes\n".to_vec())
    );
    assert_eq!(content(applied.snapshot(), "b"), None);
    assert_eq!(
        effective_workspace(applied.snapshot()).unwrap()["c"].mode,
        FileMode::Executable
    );
    assert_eq!(applied.report().base_manifest(), base.manifest());
    assert_eq!(
        applied.report().base_manifest_digest(),
        base.manifest().digest()
    );
    assert_eq!(
        applied.report().resulting_manifest_digest(),
        applied.snapshot().manifest().digest()
    );
    assert_eq!(applied.report().outcomes().len(), 4);
    assert!(
        applied
            .snapshot()
            .manifest()
            .provenance
            .starts_with("celetixo/source-application/v1")
    );
    assert_eq!(patch.apply(&base).unwrap(), applied);
}

#[test]
fn canonical_operation_order_roundtrips_and_exact_envelope_bindings_change_identity() {
    let base = base();
    let operations = vec![
        replace(&base, "a", b"new"),
        PatchOperation::Delete {
            path: "b".into(),
            expected: pre(&base, "b"),
        },
    ];
    let first = patch(&base, operations.clone());
    let second = patch(&base, operations.into_iter().rev().collect());
    assert_eq!(first, second);
    let bytes = first.canonical_bytes();
    assert_eq!(
        SourcePatch::from_canonical_bytes(&bytes, DecodeLimits::default()).unwrap(),
        first
    );
    let mutations: Vec<fn(&mut PatchBinding)> = vec![
        |b| b.scope.organization = OrganizationId::new("other").unwrap(),
        |b| b.scope.workspace = WorkspaceId::new("other").unwrap(),
        |b| b.scope.repository = RepositoryId::new("other").unwrap(),
        |b| b.principal = PrincipalId::new("other").unwrap(),
        |b| b.transaction = TransactionId::new("other").unwrap(),
        |b| b.transaction_attempt += 1,
        |b| b.source_attempt = SourceAttemptId::new("other").unwrap(),
        |b| b.base_snapshot = celetixo_engine::ids::SnapshotId(digest("other")),
        |b| b.base_revision.0 += 1,
        |b| b.analysis_artifact = digest("other"),
        |b| b.index_generation = digest("other"),
        |b| b.impact_request = digest("other"),
        |b| b.impact_result = digest("other"),
        |b| b.created_at_ms += 1,
        |b| b.expires_at_ms += 1,
        |b| b.producer.push_str(" other"),
    ];
    for mutate in mutations {
        let mut b = binding(&base);
        mutate(&mut b);
        let changed =
            SourcePatch::new(b, first.policy().clone(), first.operations().to_vec()).unwrap();
        assert_ne!(first.id(), changed.id());
        assert_eq!(first.operation_digest(), changed.operation_digest());
    }
    let mut policy = first.policy().clone();
    policy.max_result_bytes -= 1;
    let changed = SourcePatch::new(binding(&base), policy, first.operations().to_vec()).unwrap();
    assert_ne!(first.id(), changed.id());
    assert_eq!(first.operation_digest(), changed.operation_digest());
}

#[test]
fn simultaneous_rename_swaps_and_longer_cycles_have_no_order_dependence() {
    let base = base();
    for pairs in [
        vec![("a", "b"), ("b", "a")],
        vec![("a", "b"), ("b", "c"), ("c", "a")],
    ] {
        let ops: Vec<_> = pairs
            .iter()
            .map(|(from, to)| PatchOperation::Rename {
                from: (*from).into(),
                to: (*to).into(),
                expected: pre(&base, from),
            })
            .collect();
        let applied = patch(&base, ops.clone()).apply(&base).unwrap();
        assert_eq!(
            applied,
            patch(&base, ops.into_iter().rev().collect())
                .apply(&base)
                .unwrap()
        );
        for (from, to) in pairs {
            assert_eq!(content(applied.snapshot(), to), content(&base, from));
        }
    }
}

#[test]
fn conflicting_operations_and_occupied_destinations_reject_without_any_source_mutation() {
    let base = base();
    let original = base.clone();
    let rename = |from: &str, to: &str| PatchOperation::Rename {
        from: from.into(),
        to: to.into(),
        expected: pre(&base, from),
    };
    let invalid = vec![
        vec![replace(&base, "a", b"new"), replace(&base, "a", b"other")],
        vec![rename("a", "new"), rename("b", "new")],
        vec![rename("a", "new"), rename("a", "other")],
        vec![rename("a", "b"), replace(&base, "b", b"new")],
        vec![
            replace(&base, "a", b"new"),
            PatchOperation::SetMode {
                path: "a".into(),
                expected: pre(&base, "a"),
                mode: FileMode::Executable,
            },
        ],
        vec![
            PatchOperation::Delete {
                path: "a".into(),
                expected: pre(&base, "a"),
            },
            PatchOperation::Create {
                path: "a".into(),
                entry: file(b"new"),
            },
        ],
    ];
    for ops in invalid {
        assert!(matches!(
            SourcePatch::new(binding(&base), PatchPolicy::default(), ops),
            Err(SourcePatchError::ConflictingOperations(_))
        ));
    }
    assert!(matches!(
        patch(&base, vec![rename("a", "b")]).apply(&base),
        Err(SourcePatchError::OccupiedDestination(_))
    ));
    assert!(matches!(
        patch(
            &base,
            vec![PatchOperation::Create {
                path: "a".into(),
                entry: file(b"new")
            }]
        )
        .apply(&base),
        Err(SourcePatchError::OccupiedDestination(_))
    ));
    assert_eq!(base, original);
}

#[test]
fn exact_base_and_all_preimages_are_required_including_kind_and_mode() {
    let base = base();
    let original = base.clone();
    let good = patch(&base, vec![replace(&base, "a", b"new")]);
    assert_eq!(
        good.apply(&snapshot(vec![input("a", b"alpha", InputLayer::Base)]))
            .unwrap_err(),
        SourcePatchError::WrongBase
    );
    for which in 0..3 {
        let mut expected = pre(&base, "b");
        match which {
            0 => expected.content = digest("wrong"),
            1 => expected.mode = FileMode::Executable,
            _ => {
                expected.kind = InputKind::Symlink;
                expected.mode = FileMode::Symlink;
            }
        }
        let ops = vec![
            replace(&base, "a", b"would-change"),
            PatchOperation::Delete {
                path: "b".into(),
                expected,
            },
        ];
        assert!(matches!(
            patch(&base, ops).apply(&base),
            Err(SourcePatchError::PreimageMismatch(_))
        ));
    }
    for mutation in 0..3 {
        let mut binding = binding(&base);
        match mutation {
            0 => binding.scope.repository = RepositoryId::new("different").unwrap(),
            1 => binding.base_revision.0 += 1,
            _ => binding.base_snapshot = celetixo_engine::ids::SnapshotId(digest("wrong")),
        }
        let candidate =
            SourcePatch::new(binding, PatchPolicy::default(), good.operations().to_vec()).unwrap();
        assert_eq!(
            candidate.apply(&base).unwrap_err(),
            SourcePatchError::WrongBase
        );
    }
    assert_eq!(base, original);
}

#[test]
fn malformed_canonical_bytes_and_decoder_resource_amplification_fail_closed() {
    let base = base();
    let patch = patch(&base, vec![replace(&base, "a", &[0, 255, 42, 0])]);
    let bytes = patch.canonical_bytes();
    for end in [0, 1, 7, 8, 30, bytes.len() - 1] {
        assert!(SourcePatch::from_canonical_bytes(&bytes[..end], DecodeLimits::default()).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        SourcePatch::from_canonical_bytes(&trailing, DecodeLimits::default()).unwrap_err(),
        SourcePatchError::InvalidEncoding
    );
    let mut invalid_length = bytes.clone();
    invalid_length[..8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(
        SourcePatch::from_canonical_bytes(&invalid_length, DecodeLimits::default()),
        Err(SourcePatchError::Limit(_))
    ));
    let mut changed_payload = bytes.clone();
    let index = changed_payload
        .windows(4)
        .position(|v| v == [0, 255, 42, 0])
        .unwrap();
    changed_payload[index + 2] ^= 1;
    assert_eq!(
        SourcePatch::from_canonical_bytes(&changed_payload, DecodeLimits::default()).unwrap_err(),
        SourcePatchError::PayloadDigest
    );
    let mut changed_operation_digest = bytes.clone();
    *changed_operation_digest.last_mut().unwrap() ^= 1;
    assert_eq!(
        SourcePatch::from_canonical_bytes(&changed_operation_digest, DecodeLimits::default())
            .unwrap_err(),
        SourcePatchError::InvalidEncoding
    );
    for limits in [
        DecodeLimits {
            max_envelope_bytes: bytes.len() - 1,
            ..DecodeLimits::default()
        },
        DecodeLimits {
            max_operations: 0,
            ..DecodeLimits::default()
        },
        DecodeLimits {
            max_payload_bytes: 3,
            ..DecodeLimits::default()
        },
        DecodeLimits {
            max_total_payload_bytes: 3,
            ..DecodeLimits::default()
        },
    ] {
        assert!(matches!(
            SourcePatch::from_canonical_bytes(&bytes, limits),
            Err(SourcePatchError::Limit(_))
        ));
    }
    assert_eq!(
        Payload::with_digest(vec![0, 255], digest("bad")).unwrap_err(),
        SourcePatchError::PayloadDigest
    );
}

#[test]
fn noncanonical_operation_order_is_rejected_instead_of_silently_normalized_on_decode() {
    let base = base();
    // Equal-length paths and payloads let us exchange complete encoded operation
    // blocks without changing the envelope's field boundaries.
    let patch = patch(
        &base,
        vec![replace(&base, "a", b"x"), replace(&base, "b", b"y")],
    );
    let mut bytes = patch.canonical_bytes();
    fn path_offset(bytes: &[u8], path: u8) -> usize {
        let pattern = [0, 0, 0, 0, 0, 0, 0, 1, path];
        bytes
            .windows(pattern.len())
            .position(|w| w == pattern)
            .unwrap()
            - 16
    }
    let first = path_offset(&bytes, b'a');
    let second = path_offset(&bytes, b'b');
    let block_length = second - first;
    let left = bytes[first..second].to_vec();
    let right = bytes[second..second + block_length].to_vec();
    bytes[first..second].copy_from_slice(&right);
    bytes[second..second + block_length].copy_from_slice(&left);
    assert_eq!(
        SourcePatch::from_canonical_bytes(&bytes, DecodeLimits::default()).unwrap_err(),
        SourcePatchError::InvalidEncoding
    );
}

#[test]
fn portable_path_profile_rejects_traversal_control_unicode_and_windows_ambiguity() {
    let base = base();
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../b",
        "./a",
        "a//b",
        "a/",
        "a\\b",
        "C:drive",
        "nul",
        "NUL.txt",
        "NUL .txt",
        "CONIN$",
        "CONOUT$.txt",
        "CLOCK$",
        "COM1.log",
        "LPT9",
        "trailing.",
        "space ",
        "a\0b",
        "line\nb",
        "caf\u{e9}.rs",
        "cafe\u{301}.rs",
        "a?b",
        "a|b",
        ".git/config",
        "sub/.GiT/HEAD",
        ".celetixo/control",
    ] {
        assert!(
            SourcePatch::new(
                binding(&base),
                PatchPolicy::default(),
                vec![PatchOperation::Create {
                    path: path.into(),
                    entry: file(b"x")
                }]
            )
            .is_err(),
            "accepted {path:?}"
        );
    }
    let patch = patch(
        &base,
        vec![PatchOperation::Create {
            path: "valid/.github/test.rs".into(),
            entry: file(b"ok"),
        }],
    );
    assert!(patch.apply(&base).is_ok());
}

#[test]
fn final_graph_checks_case_fold_inferred_directories_and_file_ancestors() {
    let base = base();
    for operations in [
        vec![PatchOperation::Create {
            path: "A".into(),
            entry: file(b"x"),
        }],
        vec![
            PatchOperation::Create {
                path: "Upper/a".into(),
                entry: file(b"x"),
            },
            PatchOperation::Create {
                path: "upper/b".into(),
                entry: file(b"y"),
            },
        ],
        vec![PatchOperation::Create {
            path: "a/child".into(),
            entry: file(b"x"),
        }],
        vec![
            PatchOperation::Create {
                path: "dir".into(),
                entry: file(b"x"),
            },
            PatchOperation::Create {
                path: "dir/child".into(),
                entry: file(b"y"),
            },
        ],
    ] {
        assert!(matches!(
            patch(&base, operations).apply(&base),
            Err(SourcePatchError::PathCollision(_))
        ));
    }
}

#[test]
fn symlink_operations_need_explicit_policy_and_targets_are_checked_against_final_graph() {
    let base = base();
    let create_link = |path: &str, target: &[u8]| PatchOperation::Create {
        path: path.into(),
        entry: PatchEntry::symlink(target.into()).unwrap(),
    };
    assert!(matches!(
        SourcePatch::new(
            binding(&base),
            PatchPolicy::default(),
            vec![create_link("link", b"a")]
        ),
        Err(SourcePatchError::SymlinkDenied(_))
    ));
    let policy = PatchPolicy {
        symlinks: SymlinkPolicy::InternalRelativeOnly,
        ..PatchPolicy::default()
    };
    let valid = SourcePatch::new(
        binding(&base),
        policy.clone(),
        vec![
            create_link("dir/link", b"../a"),
            create_link("chain", b"dir/link"),
        ],
    )
    .unwrap()
    .apply(&base)
    .unwrap();
    assert_eq!(
        effective_workspace(valid.snapshot()).unwrap()["chain"].kind,
        InputKind::Symlink
    );
    for target in [
        b"/etc/passwd".as_slice(),
        b"../escape",
        b"missing",
        b"C:drive",
        b"a//x",
        b"a/child",
        b"a/../b",
        b"missing/../a",
        b"\xff",
    ] {
        let patch = SourcePatch::new(
            binding(&base),
            policy.clone(),
            vec![create_link("link", target)],
        )
        .unwrap();
        assert!(
            matches!(patch.apply(&base), Err(SourcePatchError::UnsafeSymlink(_))),
            "accepted {target:?}"
        );
    }
    for operations in [
        vec![create_link("x", b"y"), create_link("y", b"x")],
        vec![create_link("x", b"x")],
        vec![create_link("x", b"a"), create_link("y", b"x/child")],
        vec![
            create_link("dir/link", b"../a"),
            create_link("y", b"dir/link/../../b"),
        ],
    ] {
        assert!(matches!(
            SourcePatch::new(binding(&base), policy.clone(), operations)
                .unwrap()
                .apply(&base),
            Err(SourcePatchError::UnsafeSymlink(_))
        ));
    }
    let mut inherited = input("link", b"a", InputLayer::Base);
    inherited.kind = InputKind::Symlink;
    inherited.mode = FileMode::Symlink;
    let inherited_base = snapshot(vec![input("a", b"x", InputLayer::Base), inherited]);
    assert!(matches!(
        patch(&inherited_base, vec![replace(&inherited_base, "a", b"new")]).apply(&inherited_base),
        Err(SourcePatchError::SymlinkDenied(_))
    ));
}

#[test]
fn relative_parent_components_require_actual_directories_before_normalization() {
    let base = snapshot(vec![
        input("dir/child", b"x", InputLayer::Base),
        input("target", b"ok", InputLayer::Base),
    ]);
    let policy = PatchPolicy {
        symlinks: SymlinkPolicy::InternalRelativeOnly,
        ..PatchPolicy::default()
    };
    let operations = vec![
        PatchOperation::Create {
            path: "valid-link".into(),
            entry: PatchEntry::symlink(b"dir/../target".to_vec()).unwrap(),
        },
        PatchOperation::Create {
            path: "dir/root-link".into(),
            entry: PatchEntry::symlink(b"..".to_vec()).unwrap(),
        },
    ];
    assert!(
        SourcePatch::new(binding(&base), policy, operations)
            .unwrap()
            .apply(&base)
            .is_ok()
    );
}

#[test]
fn all_layer_provenance_uncertainty_environment_and_exclusions_are_retained() {
    let mut manifest = base().manifest().clone();
    manifest.inputs.extend([
        input("a", b"staged", InputLayer::Index),
        input("a", b"working", InputLayer::WorkingTree),
        input("a", b"editor", InputLayer::Overlay),
        input("gen/out", b"generated", InputLayer::Generated),
        input("mounted/input", b"mounted", InputLayer::Mounted),
        input(".git/config", b"[core]\n", InputLayer::Mounted),
    ]);
    manifest.environment.insert(
        "celetixo.capture.uncertainty".into(),
        digest("LiveSourceNotAtomic").to_string(),
    );
    manifest.environment.insert(
        "celetixo.capture.consistency".into(),
        "observed-two-pass/v1".into(),
    );
    manifest
        .environment
        .insert("project.config".into(), "retained original input".into());
    manifest
        .environment
        .insert("celetixo.custom".into(), "execution configuration".into());
    manifest.environment.insert(
        "celetixo.capture.future".into(),
        "unclassified input".into(),
    );
    manifest.inclusion_policy.excluded_paths.insert(
        "excluded".into(),
        "missing relevant input remains uncertain".into(),
    );
    let base = Snapshot::new(scope(), Revision(7), manifest, State::new()).unwrap();
    let original = base.clone();
    let applied = patch(&base, vec![replace(&base, "a", b"patched")])
        .apply(&base)
        .unwrap();
    assert_eq!(applied.report().base_manifest(), base.manifest());
    assert_eq!(applied.report().non_operational_inputs().len(), 1);
    assert_eq!(
        applied.report().non_operational_inputs()[0].input.path,
        ".git/config"
    );
    assert!(
        !applied.report().non_operational_inputs()[0]
            .reason
            .is_empty()
    );
    assert_eq!(content(applied.snapshot(), ".git/config"), None);
    assert_eq!(
        content(applied.snapshot(), "gen/out"),
        Some(b"generated".to_vec())
    );
    assert_eq!(
        content(applied.snapshot(), "mounted/input"),
        Some(b"mounted".to_vec())
    );
    assert_eq!(content(applied.snapshot(), "a"), Some(b"patched".to_vec()));
    assert_eq!(
        applied
            .snapshot()
            .manifest()
            .inclusion_policy
            .excluded_paths,
        base.manifest().inclusion_policy.excluded_paths
    );
    assert_eq!(
        applied.snapshot().manifest().environment["celetixo.source_application.origin.celetixo.capture.uncertainty"],
        base.manifest().environment["celetixo.capture.uncertainty"]
    );
    assert!(
        !applied
            .snapshot()
            .manifest()
            .environment
            .contains_key("celetixo.capture.consistency")
    );
    assert_eq!(
        applied.snapshot().manifest().environment["project.config"],
        "retained original input"
    );
    assert_eq!(
        applied.snapshot().manifest().environment["celetixo.custom"],
        "execution configuration"
    );
    assert_eq!(
        applied.snapshot().manifest().environment["celetixo.capture.future"],
        "unclassified input"
    );
    assert_eq!(base, original);
    let candidate = patch(
        &base,
        vec![PatchOperation::Create {
            path: "excluded/new".into(),
            entry: file(b"x"),
        }],
    );
    assert!(matches!(
        candidate.apply(&base),
        Err(SourcePatchError::ExcludedPath(_))
    ));
}

#[test]
fn arbitrary_reserved_base_content_cannot_be_silently_omitted_as_metadata() {
    for (path, layer) in [
        (".celetixo/user-data", InputLayer::Mounted),
        (".git/user-data", InputLayer::Base),
        (".git/user-data", InputLayer::Mounted),
    ] {
        let base = snapshot(vec![
            input("a", b"x", InputLayer::Base),
            input(path, b"must not disappear", layer),
        ]);
        assert!(matches!(
            patch(&base, vec![replace(&base, "a", b"new")]).apply(&base),
            Err(SourcePatchError::ReservedPath(_))
        ));
    }
}

#[test]
fn unknown_reserved_application_environment_rejects_and_prior_uncertainty_survives_reapplication() {
    let mut manifest = base().manifest().clone();
    manifest.environment.insert(
        "celetixo.source_application.unknown".into(),
        "cannot silently drop".into(),
    );
    let reserved = Snapshot::new(scope(), Revision(7), manifest, State::new()).unwrap();
    assert!(matches!(
        patch(&reserved, vec![replace(&reserved, "a", b"x")]).apply(&reserved),
        Err(SourcePatchError::ReservedEnvironment(_))
    ));
    let mut manifest = base().manifest().clone();
    manifest.environment.insert(
        "celetixo.capture.uncertainty".into(),
        "live capture remains uncertain".into(),
    );
    let original = Snapshot::new(scope(), Revision(7), manifest, State::new()).unwrap();
    let first = patch(&original, vec![replace(&original, "a", b"first")])
        .apply(&original)
        .unwrap();
    let second = patch(
        first.snapshot(),
        vec![replace(first.snapshot(), "a", b"second")],
    )
    .apply(first.snapshot())
    .unwrap();
    assert_eq!(
        second.snapshot().manifest().environment["celetixo.source_application.origin.celetixo.capture.uncertainty"],
        "live capture remains uncertain"
    );
    assert_eq!(second.report().base_manifest(), first.snapshot().manifest());
}

#[test]
fn noops_empty_patches_invalid_bindings_and_result_limits_are_explicit() {
    let base = base();
    assert_eq!(
        SourcePatch::new(binding(&base), PatchPolicy::default(), vec![]).unwrap_err(),
        SourcePatchError::Empty
    );
    assert!(matches!(
        patch(&base, vec![replace(&base, "a", b"alpha")]).apply(&base),
        Err(SourcePatchError::NoOp(_))
    ));
    let identical = snapshot(vec![
        input("a", b"same", InputLayer::Base),
        input("b", b"same", InputLayer::Base),
    ]);
    let swap = patch(
        &identical,
        vec![
            PatchOperation::Rename {
                from: "a".into(),
                to: "b".into(),
                expected: pre(&identical, "a"),
            },
            PatchOperation::Rename {
                from: "b".into(),
                to: "a".into(),
                expected: pre(&identical, "b"),
            },
        ],
    );
    assert_eq!(swap.apply(&identical).unwrap_err(), SourcePatchError::Empty);
    for which in 0..3 {
        let mut b = binding(&base);
        match which {
            0 => b.expires_at_ms = b.created_at_ms,
            1 => b.producer.clear(),
            _ => b.producer.push('\n'),
        }
        assert_eq!(
            SourcePatch::new(b, PatchPolicy::default(), vec![replace(&base, "a", b"x")])
                .unwrap_err(),
            SourcePatchError::InvalidBinding
        );
    }
    for policy in [
        PatchPolicy {
            max_result_files: 1,
            ..PatchPolicy::default()
        },
        PatchPolicy {
            max_result_bytes: 1,
            ..PatchPolicy::default()
        },
    ] {
        assert!(matches!(
            SourcePatch::new(binding(&base), policy, vec![replace(&base, "a", b"x")])
                .unwrap()
                .apply(&base),
            Err(SourcePatchError::Limit(_))
        ));
    }
    let policy = PatchPolicy {
        max_payload_bytes: 1,
        ..PatchPolicy::default()
    };
    assert!(matches!(
        SourcePatch::new(binding(&base), policy, vec![replace(&base, "a", b"xx")]),
        Err(SourcePatchError::Limit(_))
    ));
}

#[test]
fn replay_validation_rejects_an_application_from_a_different_patch() {
    let base = base();
    let patch_a = patch(&base, vec![replace(&base, "a", b"one")]);
    let patch_b = patch(&base, vec![replace(&base, "a", b"two")]);
    let applied = patch_a.apply(&base).unwrap();
    assert_eq!(
        applied.validate(&base, &patch_b),
        Err(SourcePatchError::InvalidReport)
    );
    assert_ne!(
        applied.report().digest(),
        patch_b.apply(&base).unwrap().report().digest()
    );
}

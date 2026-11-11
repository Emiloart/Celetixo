//! Read-only actual-repository T0 measurements. No repository program is run,
//! and pure patch bindings here are explicitly not Engine execution admission.
mod bench_support;
use bench_support::{environment, memory, micros};
use celetixo_engine::ids::*;
use celetixo_engine::snapshot::{FileMode, InputKind, Snapshot, effective_workspace};
use celetixo_engine::source_patch::*;
use celetixo_execution::materialize::{CleanupOutcome, PrivateWorkspace};
use celetixo_repository::capture::{CaptureRequest, capture_git_tree};
use celetixo_repository::reviewed_inputs::add_exact_review;
use serde_json::{Value, json};
use std::error::Error;
use std::time::Instant;

fn identity(name: &str) -> Digest {
    Digest::of("celetixo/t0-benchmark-not-an-admission/v1", name.as_bytes())
}

fn binding(base: &Snapshot) -> PatchBinding {
    PatchBinding {
        scope: base.scope().clone(),
        principal: PrincipalId::new("benchmark-operator").unwrap(),
        transaction: TransactionId::new("t0-measurement-not-an-execution").unwrap(),
        transaction_attempt: 1,
        source_attempt: SourceAttemptId::new("t0-measurement").unwrap(),
        base_snapshot: base.id(),
        base_revision: base.revision(),
        analysis_artifact: identity("no-analysis-admission"),
        index_generation: identity("no-index-admission"),
        impact_request: identity("no-impact-request"),
        impact_result: identity("no-complete-impact"),
        created_at_ms: 0,
        expires_at_ms: 1,
        producer: "pure T0 benchmark; no Engine admission or semantic completeness".into(),
    }
}

fn measure(
    base: &Snapshot,
    name: &str,
    operation: PatchOperation,
) -> Result<Value, Box<dyn Error>> {
    let start = Instant::now();
    let patch = SourcePatch::new(binding(base), PatchPolicy::default(), vec![operation])?;
    let canonical = patch.canonical_bytes();
    let encode_us = micros(start);
    let start = Instant::now();
    let decoded = SourcePatch::from_canonical_bytes(&canonical, DecodeLimits::default())?;
    assert_eq!(decoded, patch);
    let decode_us = micros(start);
    let start = Instant::now();
    let applied = patch.apply(base)?;
    let apply_us = micros(start);
    let start = Instant::now();
    applied.validate(base, &patch)?;
    let replay_us = micros(start);
    let start = Instant::now();
    let manifest = applied.snapshot().manifest().digest();
    let manifest_us = micros(start);
    let start = Instant::now();
    let workspace = PrivateWorkspace::materialize(base, &patch)?;
    let materialize_us = micros(start);
    assert_eq!(workspace.applied(), &applied);
    let report = workspace.report().clone();
    let start = Instant::now();
    workspace.validate_contents()?;
    let census_us = micros(start);
    let observed_memory = memory();
    let start = Instant::now();
    assert_eq!(workspace.cleanup(), CleanupOutcome::Removed);
    let cleanup_us = micros(start);
    Ok(
        json!({"case":name,"operation":format!("{:?}",patch.operations()[0].kind()),
        "patch":patch.id().to_string(),"patch_encoded_bytes":canonical.len(),
        "candidate":applied.snapshot().id().to_string(),"manifest":manifest.to_string(),
        "patch_canonicalization_and_encode_us":encode_us,"patch_decode_us":decode_us,
        "patch_apply_us":apply_us,"application_replay_validation_us":replay_us,
        "manifest_canonical_and_hash_us":manifest_us,"private_materialization_including_replay_us":materialize_us,
        "extra_source_census_us":census_us,"source_cleanup_us":cleanup_us,
        "source_files":report.files(),"source_symlinks":report.symlinks(),"source_directories":report.directories(),
        "source_content_bytes_copied":report.content_bytes(),"source_content_bytes_reused":0,
        "source_copy_bytes_per_second":if materialize_us==0 {None} else {Some(report.content_bytes() as f64*1_000_000.0/materialize_us as f64)},
        "host_memory":observed_memory,"engine_execution_admitted":false}),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 || (args.len() - 2) % 2 != 0 {
        return Err("usage: source_patch_bench ROOT FULL_GIT_OID [--iterations 1..20] [--reviewed-public-input PATH=CONTENT_DIGEST]...".into());
    }
    let (root, oid) = (&args[0], &args[1]);
    let mut iterations = 3;
    let mut selected_iterations = false;
    let mut credentials = celetixo_engine::credentials::CredentialPolicy::default();
    for pair in args[2..].chunks_exact(2) {
        match pair[0].as_str() {
            "--iterations" if !selected_iterations => {
                iterations = pair[1].parse::<usize>()?;
                selected_iterations = true;
            }
            "--reviewed-public-input" => add_exact_review(&mut credentials, &pair[1])?,
            _ => return Err("unknown or repeated benchmark option".into()),
        }
    }
    if !(1..=20).contains(&iterations) {
        return Err("iterations must be 1..20".into());
    }
    let scope = Scope {
        organization: OrganizationId::new("benchmark")?,
        workspace: WorkspaceId::new("t0")?,
        repository: RepositoryId::new("selected-git-tree")?,
    };
    let mut samples = Vec::new();
    let mut first_snapshot = None;
    for iteration in 0..iterations {
        let start = Instant::now();
        let mut request = CaptureRequest::new(root, scope.clone(), Revision(0));
        request.policy.credentials = credentials.clone();
        let captured = capture_git_tree(&request, oid)?;
        let capture_us = micros(start);
        if !captured.report.complete() {
            return Err("incomplete capture is not a benchmark input".into());
        }
        let base = &captured.snapshot;
        if let Some(previous) = first_snapshot.replace(base.id()) {
            assert_eq!(
                previous,
                base.id(),
                "pinned Git capture changed between repetitions"
            );
        }
        let original = base.canonical_bytes();
        let effective = effective_workspace(base)?;
        let file = effective
            .values()
            .find(|file| file.kind == InputKind::File)
            .ok_or("no regular source file")?;
        let preimage = Preimage::of(file)?;
        let mut replaced = file.bytes.clone().ok_or("missing bytes")?;
        replaced.push(b'\n');
        let mut cases = vec![
            (
                "replace-original",
                PatchOperation::Replace {
                    path: file.path.clone(),
                    expected: preimage,
                    entry: PatchEntry::file(replaced, file.mode)?,
                },
            ),
            (
                "delete-original",
                PatchOperation::Delete {
                    path: file.path.clone(),
                    expected: preimage,
                },
            ),
            (
                "rename-original",
                PatchOperation::Rename {
                    from: file.path.clone(),
                    to: "benchmark-renamed-input.txt".into(),
                    expected: preimage,
                },
            ),
            (
                "mode-original",
                PatchOperation::SetMode {
                    path: file.path.clone(),
                    expected: preimage,
                    mode: if file.mode == FileMode::Regular {
                        FileMode::Executable
                    } else {
                        FileMode::Regular
                    },
                },
            ),
        ];
        for (name, size) in [
            ("create-4KiB", 4096),
            ("create-64KiB", 65536),
            ("create-1MiB", 1048576),
        ] {
            cases.push((
                name,
                PatchOperation::Create {
                    path: "benchmark-payload.bin".into(),
                    entry: PatchEntry::file(vec![0x5a; size], FileMode::Regular)?,
                },
            ));
        }
        let cases = cases
            .into_iter()
            .map(|(name, op)| measure(base, name, op))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(base.canonical_bytes(), original);
        samples.push(json!({"iteration":iteration,"git_commit":oid,"base_snapshot":base.id().to_string(),
            "base_manifest":base.manifest().digest().to_string(),"capture_us":capture_us,
            "capture_policy":captured.report.policy_digest.to_string(),"capture_complete":true,
            "base_source_files":effective.len(),"base_source_bytes":effective.values().map(|f|f.bytes.as_ref().map_or(0,Vec::len)).sum::<usize>(),
            "cases":cases}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema":"celetixo/source-patch-benchmark/v1","input_kind":"actual selected immutable Git repository",
            "benchmark_source":Digest::of("celetixo/source-content/v1",include_bytes!("source_patch_bench.rs")).to_string(),
            "observation_source":Digest::of("celetixo/source-content/v1",include_bytes!("bench_support/mod.rs")).to_string(),
            "environment":environment(),"iterations":iterations,"samples":samples,
            "execution_tier":"T0 pure patching and no-follow filesystem copying only; no repository code or dependency installation",
            "analysis_scope":"no semantic completeness or Engine execution admission asserted; benchmark-only patch bindings",
            "timing_scope":"independent probes; materialization includes its own replay/census; do not sum overlapping probes",
            "workspace_scope":"standalone ephemeral T0 copies, removed after each probe; actual execution requires the scoped registry"
        }))?
    );
    Ok(())
}

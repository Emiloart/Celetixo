//! Measured finite reference execution through the real, reviewed OCI backend.
//! Acquiring dependencies is a separate operator-controlled provisioning step.
mod bench_support;
#[path = "bench_support/execution_recovery.rs"]
mod execution_recovery;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../tests/support/mod.rs"]
mod fixture;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use celetixo_engine::ids::Digest;
    use celetixo_engine::source_evidence::{SourceArtifactReader, SourceEvidenceAssessment};
    use celetixo_engine::source_execution::{SourceCacheContext, SourceCacheKey};
    use celetixo_execution::artifacts::{ArtifactStoreLimits, LocalSourceArtifactStore};
    use celetixo_execution::coordinator::{
        MonotonicEngineClock, SourceRunCoordinator, SourceRunRequest,
    };
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Instant;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let iterations: usize = match args.as_slice() {
        [] => 3,
        [flag, n] if flag == "--iterations" => n.parse()?,
        _ => return Err("usage: source_execution_bench [--iterations 1..20]".into()),
    };
    if !(1..=20).contains(&iterations) {
        return Err("iterations must be 1..20".into());
    }
    let backend = fixture::real_backend();
    let mut samples = Vec::new();
    let mut case_bases = std::collections::BTreeMap::new();
    for iteration in 0..iterations {
        for (case, source_bytes, asset_count) in [
            ("4KiB-source", 4096, 0),
            ("64KiB-source", 65536, 0),
            ("64KiB-with-128-assets", 65536, 128),
        ] {
            let base_source = generated_source(source_bytes, 41);
            let candidate = generated_source(source_bytes, 42);
            let mut assets = vec![(
                "go.mod".into(),
                b"module example.invalid/fixture\n\ngo 1.26.2\n".to_vec(),
            )];
            for index in 0..asset_count {
                assets.push((format!("assets/input{index:04}.txt"), vec![b'x'; 256]));
            }
            let mut resources = fixture::limits();
            resources.artifact_bytes = 8 * 1024 * 1024;
            let fixture_started = Instant::now();
            let f = fixture::admitted_source(
                &backend,
                &candidate,
                "/toolchain/gofmt",
                &["-l", "/src/main.go"],
                resources,
                &base_source,
                &assets,
            );
            let admission_us = fixture_started.elapsed().as_micros();
            if let Some(previous) = case_bases.insert(case, f.base.id()) {
                assert_eq!(
                    previous,
                    f.base.id(),
                    "fixture capture changed between equal cases"
                );
            }
            let start = Instant::now();
            let applied = f.patch.apply(&f.base)?;
            let patch_apply_us = bench_support::micros(start);
            let start = Instant::now();
            applied.validate(&f.base, &f.patch)?;
            let patch_replay_us = bench_support::micros(start);
            let start = Instant::now();
            let manifest = applied.snapshot().manifest().digest();
            let manifest_hash_us = bench_support::micros(start);
            assert_eq!(manifest, f.admission.resulting_manifest());
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("persisted-evidence");
            let limits = ArtifactStoreLimits {
                max_object_bytes: 16 * 1024 * 1024,
                max_total_bytes: 64 * 1024 * 1024,
                max_objects: 256,
            };
            let artifacts = Arc::new(LocalSourceArtifactStore::create(
                &path,
                fixture::scope(),
                f.admission.retention_policy(),
                limits,
            )?);
            let storage = f
                .admin
                .register_source_artifact_store(fixture::scope(), artifacts.clone())?;
            let mut clock = MonotonicEngineClock::new(f.admin.clone());
            let sample = {
                let workspaces = fixture::workspace_registry();
                let coordinator = SourceRunCoordinator {
                    engine: &f.engine,
                    executor: &f.executor,
                    backend: &backend,
                    artifacts: artifacts.as_ref(),
                    storage: &storage,
                    workspaces: &workspaces,
                };
                let result = coordinator.execute(
                    SourceRunRequest {
                        base: &f.base,
                        patch: &f.patch,
                        policy: &f.policy,
                        admission: &f.admission,
                    },
                    &mut clock,
                )?;
                if result.receipt.assessment != SourceEvidenceAssessment::Passed {
                    return Err(format!(
                        "reference execution did not pass: {:?}",
                        result.receipt.assessment
                    )
                    .into());
                }
                let query = Instant::now();
                let retained = f.engine.source_evidence_record(
                    &f.owner,
                    &fixture::scope(),
                    result.receipt.evidence,
                )?;
                let receipt_query_us = query.elapsed().as_micros();
                if retained.1 != result.receipt {
                    return Err("historical receipt differs".into());
                }
                let read = Instant::now();
                let bytes = artifacts.read(
                    &fixture::scope(),
                    result.sealed.envelope.content,
                    result.sealed.envelope.bytes,
                )?;
                if bytes != result.sealed.evidence.canonical_bytes() {
                    return Err("sealed bytes differ".into());
                }
                let envelope_read_us = read.elapsed().as_micros();
                let record = result.sealed.evidence.checks()[f.definition.id()].input();
                let raw_ref = &record.artifacts["isolation-record"];
                let raw: serde_json::Value = serde_json::from_slice(&artifacts.read(
                    &fixture::scope(),
                    raw_ref.content,
                    raw_ref.bytes,
                )?)?;
                let stored_bytes: u64 = result
                    .sealed
                    .evidence
                    .provenance()
                    .values()
                    .chain(
                        result
                            .sealed
                            .evidence
                            .checks()
                            .values()
                            .flat_map(|c| c.input().artifacts.values()),
                    )
                    .map(|r| r.bytes)
                    .sum::<u64>()
                    + result.sealed.envelope.bytes;
                let cache_start = Instant::now();
                let context = SourceCacheContext {
                    platform: "linux".into(),
                    architecture: "x86_64".into(),
                    external_inputs: Default::default(),
                };
                let key =
                    SourceCacheKey::for_check(f.admission.plan(), f.definition.id(), &context)?;
                let cache_key_us = cache_start.elapsed().as_micros();
                json!({
                    "iteration":iteration,"case":case,"source_bytes":source_bytes,"asset_count":asset_count,"transaction":f.transaction.as_str(),"base_snapshot":f.base.id().to_string(),"candidate":f.admission.resulting_snapshot().to_string(),
                    "patch":f.patch.id().to_string(),"policy":f.policy.digest().to_string(),"plan":f.admission.plan().id().to_string(),
                    "check_definition":f.definition.digest().to_string(),"evidence":result.receipt.evidence.to_string(),
                    "fixture_and_admission_us":admission_us,"patch_apply_us":patch_apply_us,
                    "patch_replay_validation_us":patch_replay_us,"manifest_canonical_and_hash_us":manifest_hash_us,
                    "materialized_files":result.materialization.files(),"copied_source_content_bytes":result.materialization.content_bytes(),"reused_source_content_bytes":0,
                    "host_memory":bench_support::memory(),
                    "materialization_us":result.timings.materialization_us,"checks_including_backend_setup_us":result.timings.checks_us,
                    "source_cleanup_us":result.timings.source_cleanup_us,"persistence_us":result.timings.persistence_us,
                    "engine_evidence_admission_us":result.timings.evidence_admission_us,"total_run_us":result.timings.total_us,
                    "receipt_query_us":receipt_query_us,"envelope_read_us":envelope_read_us,"cache_key_us":cache_key_us,
                    "cache_key":key.digest().to_string(),"cache_reuse_enabled":key.reuse_enabled(),
                    "stored_reference_bytes_including_duplicate_references":stored_bytes,
                    "recorded_wall_time_ms":record.usage.wall_time_ms,"recorded_cpu_time_us":record.usage.cpu_time_us,
                    "recorded_peak_memory_bytes":record.usage.peak_memory_bytes,
                    "raw_execution_and_effective_isolation":raw,
                })
            };
            samples.push(sample);
            drop(clock);
            drop(storage);
            drop(f);
            drop(artifacts);
        }
    }
    let mut recovery = Vec::new();
    for iteration in 0..iterations {
        recovery.push(execution_recovery::measure(&backend, iteration)?);
    }
    let source = include_bytes!("source_execution_bench.rs");
    let fixture_source = include_bytes!("../tests/support/mod.rs");
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema":"celetixo/source-execution-benchmark/v2","fixture":"generated-enumerated-go-syntax/v2",
            "build_profile":if cfg!(debug_assertions){"debug"}else{"release"},
            "benchmark_source":Digest::of("celetixo/source-content/v1",source).to_string(),
            "fixture_source":Digest::of("celetixo/source-content/v1",fixture_source).to_string(),
            "observation_source":Digest::of("celetixo/source-content/v1",include_bytes!("bench_support/mod.rs")).to_string(),
            "backend":backend.identity().digest().to_string(),"toolchain":"gofmt/go1.26.2; explicitly pinned helper, loader and libc",
            "kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease")?.trim(),"architecture":std::env::consts::ARCH,
            "cpu":std::fs::read_to_string("/proc/cpuinfo")?.lines().find(|s|s.starts_with("model name")).unwrap_or("unknown"),
            "rustc_and_tooling_provenance":"record separately with the exact source commit and reviewed provisioning manifest",
            "iterations":iterations,"samples":samples,"capacity_and_recovery":recovery,"cold_cache_claim":false,
            "environment":bench_support::environment(),
            "recovery_source":Digest::of("celetixo/source-content/v1",include_bytes!("bench_support/execution_recovery.rs")).to_string(),
            "queue_policy":"synchronous; a busy backend rejects rather than queues; no queue latency measured",
            "cache_policy":"reuse disabled; complete key construction validates plan/check/context then records no reusable hit",
            "fixture_policy":"fixed fixture Git dates; source/asset sizes and bytes are deterministic; attempts and evidence retain distinct identities",
            "scope":"real OCI execution and persisted evidence for finite reference analysis; no complete Go semantic-support claim"
        }))?
    );
    Ok(())
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("requires Linux x86_64 with reviewed OCI tooling and delegated cgroupv2");
    std::process::exit(1);
}

fn generated_source(bytes: usize, answer: u8) -> Vec<u8> {
    let prefix = format!("package main\n\nfunc answer() int {{ return {answer} }}\n\n/*");
    let mut source = prefix.into_bytes();
    source.resize(bytes - 3, b'x');
    source.extend_from_slice(b"*/\n");
    assert_eq!(source.len(), bytes);
    source
}

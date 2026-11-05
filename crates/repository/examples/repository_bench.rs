//! Reproducible, bounded exploratory measurements; never an authorization gate.
//! Run from a source checkout. No benchmark source is compiled or executed.
use celetixo_engine::access::AccessSet;
use celetixo_engine::ids::{OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_repository::capture::{
    CaptureRequest, CaptureResult, Overlay, capture, capture_git_tree,
};
use celetixo_repository::impact::{ImpactAnalyzer, ImpactRequest, measure_quality};
use celetixo_repository::index::{IndexConfig, Indexer, Language, SourceIndex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Reference {
    version: String,
    unit: String,
    cases: Vec<ReferenceCase>,
}
#[derive(Deserialize)]
struct ReferenceCase {
    name: String,
    changed_path: String,
    changed_symbol: String,
    affected_files: BTreeSet<String>,
    rationale: String,
}
#[derive(Serialize)]
struct Quality {
    case: String,
    changed_element: String,
    predicted_files: BTreeSet<String>,
    reference_files: BTreeSet<String>,
    missed_files: BTreeSet<String>,
    extra_files: BTreeSet<String>,
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    incomplete: bool,
    rationale: String,
}
#[derive(Serialize)]
struct Memory {
    /// Linux /proc/self/status values, KiB. None on unsupported platforms.
    rss_kib: Option<u64>,
    /// Process lifetime high-water mark, not a phase allocation measurement.
    lifetime_peak_rss_kib: Option<u64>,
}
#[derive(Serialize)]
struct Sample {
    iteration: usize,
    clean_snapshot: String,
    immutable_git_snapshot: String,
    dirty_snapshot: String,
    clean_generation: String,
    dirty_generation: String,
    configuration: String,
    effective_files: usize,
    source_files: BTreeMap<String, usize>,
    effective_bytes: usize,
    clean_capture_manifest_inputs: usize,
    dirty_capture_manifest_inputs: usize,
    nodes: usize,
    edges: usize,
    issues: usize,
    changed_nodes: usize,
    changed_edges: usize,
    changed_issues: usize,
    changed_effective_bytes: usize,
    capture_clean_ns: u64,
    capture_repeat_ns: u64,
    capture_git_tree_ns: u64,
    capture_dirty_ns: u64,
    cold_index_ns: u64,
    incremental_index_ns: u64,
    rebuild_index_ns: u64,
    graph_prepare_ns: u64,
    impact_query_complete: bool,
    impact_query_affected_files: usize,
    impact_query_gap_count: usize,
    cached_impact_query_ns: Vec<u64>,
    cold_parsed_files: usize,
    incremental_parsed_files: usize,
    incremental_reused_files: usize,
    rebuild_equivalent: bool,
    repeat_capture_equivalent: bool,
    live_capture_complete: Option<bool>,
    live_capture_census_complete: Option<bool>,
    immutable_git_capture_complete: bool,
    memory_before: Memory,
    memory_after_cold: Memory,
    memory_after_incremental_and_rebuild: Memory,
    memory_after_queries: Memory,
    quality: Vec<Quality>,
}
#[derive(Serialize)]
struct Corpus {
    fixture: &'static str,
    reference: Option<String>,
    reference_unit: Option<String>,
    copies: Option<usize>,
    repository_commit: Option<String>,
    change_path: Option<String>,
    capture_mode: &'static str,
    fixture_content_digest: Option<String>,
    manual_reference_digest: Option<String>,
}
#[derive(Serialize)]
struct Report {
    schema: &'static str,
    #[serde(flatten)]
    corpus: Corpus,
    iterations: usize,
    queries_per_iteration: usize,
    build_profile: &'static str,
    native_cflags_at_build: Option<&'static str>,
    crate_cc_no_defaults_at_build: Option<&'static str>,
    /// Explicitly supplied from `rustc --version` before compiling this example.
    rustc_version_recorded_at_build: Option<&'static str>,
    native_compiler_recorded_at_build: Option<&'static str>,
    rustup_toolchain_at_build: Option<&'static str>,
    analyzer: String,
    parsers: BTreeSet<String>,
    /// Compiled capture/index/impact/harness source plus Cargo.lock, not Git HEAD.
    measurement_implementation_digest: String,
    git_version: String,
    operating_system: String,
    architecture: &'static str,
    cpu: Option<String>,
    capture_filesystem: Option<String>,
    samples: Vec<Sample>,
}

struct PreparedCapture {
    clean: CaptureResult,
    immutable: CaptureResult,
    capture_clean_ns: u64,
    capture_repeat_ns: u64,
    capture_git_tree_ns: u64,
    memory_before: Memory,
    live: bool,
}

fn nanos(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).expect("bounded benchmark duration")
}

fn memory() -> Memory {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let value = |key: &str| {
        status.lines().find_map(|line| {
            line.strip_prefix(key)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
    };
    Memory {
        rss_kib: value("VmRSS:"),
        lifetime_peak_rss_kib: value("VmHWM:"),
    }
}

fn implementation_digest() -> String {
    let mut encoded = Vec::new();
    for (path, content) in [
        (
            "Cargo.lock",
            include_bytes!("../../../Cargo.lock").as_slice(),
        ),
        ("capture.rs", include_bytes!("../src/capture.rs").as_slice()),
        ("index.rs", include_bytes!("../src/index.rs").as_slice()),
        ("impact.rs", include_bytes!("../src/impact.rs").as_slice()),
        (
            "repository_bench.rs",
            include_bytes!("repository_bench.rs").as_slice(),
        ),
    ] {
        encoded.extend_from_slice(&(path.len() as u64).to_be_bytes());
        encoded.extend_from_slice(path.as_bytes());
        encoded.extend_from_slice(&(content.len() as u64).to_be_bytes());
        encoded.extend_from_slice(content);
    }
    celetixo_engine::ids::Digest::of("celetixo/benchmark-implementation/v1", &encoded).to_string()
}

fn filesystem_type(path: &Path) -> Option<String> {
    let output = Command::new("stat")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["-f", "-c", "%T", "--"])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().into())
}

/// Only the self-authored fixture is passed to Git. Ignore all ambient Git config,
/// hooks and identity variables; fixture commit identity and date are fixed.
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Celetixo benchmark",
            "-c",
            "user.email=benchmark@example.invalid",
        ])
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "fixture git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn fixture_inputs(root: &Path, path: &Path, inputs: &mut BTreeMap<String, String>) -> Result<()> {
    let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fixture_inputs(root, &path, inputs)?;
        } else if entry.file_type()?.is_file() {
            inputs.insert(
                path.strip_prefix(root)?
                    .to_str()
                    .ok_or("non-UTF8 fixture path")?
                    .replace('\\', "/"),
                fs::read_to_string(&path)?,
            );
        } else {
            return Err("benchmark fixture must contain only regular files".into());
        }
    }
    Ok(())
}

fn materialize(root: &Path, templates: &BTreeMap<String, String>, copies: usize) -> Result<()> {
    for (template_path, template) in templates {
        let count = if template_path.contains("__UNIT__") {
            copies
        } else {
            1
        };
        for copy in 0..count {
            let unit = format!("unit{copy:03}");
            let path = root.join(template_path.replace("__UNIT__", &unit));
            fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
            fs::write(path, template.replace("__UNIT__", &unit))?;
        }
    }
    fs::write(
        root.join("src/lib.rs"),
        (0..copies)
            .map(|copy| format!("pub mod unit{copy:03};\n"))
            .collect::<String>(),
    )?;
    Ok(())
}

fn requests(index: &SourceIndex, reference: &Reference) -> Result<Vec<ImpactRequest>> {
    reference
        .cases
        .iter()
        .map(|case| {
            let matching: Vec<_> = index
                .symbols(&case.changed_symbol)
                .into_iter()
                .filter(|node| node.path == case.changed_path)
                .collect();
            if matching.len() != 1 {
                return Err(format!(
                    "manual case {} needs exactly one declaration, got {}",
                    case.name,
                    matching.len()
                )
                .into());
            }
            Ok(ImpactRequest {
                snapshot: index.snapshot,
                generation: index.generation,
                changed: BTreeSet::from([matching[0].id.clone()]),
                transaction_accesses: AccessSet::default(),
                invariants: vec![],
            })
        })
        .collect()
}

fn run_iteration(
    iteration: usize,
    copies: usize,
    queries: usize,
    templates: &BTreeMap<String, String>,
    reference: &Reference,
) -> Result<Sample> {
    let repository = tempfile::tempdir()?;
    materialize(repository.path(), templates, copies)?;
    git(
        repository.path(),
        &[
            "init",
            "-q",
            "--object-format=sha1",
            "--initial-branch=bench",
        ],
    )?;
    git(repository.path(), &["add", "."])?;
    git(
        repository.path(),
        &["commit", "-qm", "deterministic benchmark fixture v1"],
    )?;
    let head = git(repository.path(), &["rev-parse", "HEAD"])?;
    let request = CaptureRequest::new(
        repository.path(),
        Scope {
            organization: OrganizationId::new("benchmark-org")?,
            workspace: WorkspaceId::new("benchmark-workspace")?,
            repository: RepositoryId::new("benchmark-repository")?,
        },
        Revision(0),
    );
    let memory_before = memory();
    let start = Instant::now();
    let clean = capture(&request)?;
    let capture_clean_ns = nanos(start);
    let start = Instant::now();
    let repeated = capture(&request)?;
    let capture_repeat_ns = nanos(start);
    if clean != repeated {
        return Err("repeated clean capture changed".into());
    }
    let start = Instant::now();
    let immutable = capture_git_tree(&request, &head)?;
    let capture_git_tree_ns = nanos(start);
    let prepared = PreparedCapture {
        clean,
        immutable,
        capture_clean_ns,
        capture_repeat_ns,
        capture_git_tree_ns,
        memory_before,
        live: true,
    };
    measure_pair(iteration, queries, prepared, Some(reference), None, || {
        // A staged edit, an additional unstaged edit, and an untracked config.
        // Only one source file changes, so exactly one syntax tree is reparsed.
        let changed = repository.path().join("typescript/unit000/math.ts");
        fs::write(
            &changed,
            "export function base(x: number): number { return x + 2; }\n",
        )?;
        git(repository.path(), &["add", "typescript/unit000/math.ts"])?;
        fs::write(
            &changed,
            "export function base(x: number): number { return x + 3; }\n",
        )?;
        fs::write(
            repository.path().join("benchmark-untracked.json"),
            "{\"benchmark\":1}\n",
        )?;
        let start = Instant::now();
        let dirty = capture(&request)?;
        Ok((dirty, nanos(start)))
    })
}

fn measure_pair(
    iteration: usize,
    queries: usize,
    prepared: PreparedCapture,
    reference: Option<&Reference>,
    change_path: Option<&str>,
    capture_changed: impl FnOnce() -> Result<(CaptureResult, u64)>,
) -> Result<Sample> {
    let PreparedCapture {
        clean,
        immutable,
        capture_clean_ns,
        capture_repeat_ns,
        capture_git_tree_ns,
        memory_before,
        live,
    } = prepared;
    let mut indexer = Indexer::new(IndexConfig::default())?;
    let start = Instant::now();
    let index = indexer.build(&clean.snapshot)?;
    let cold_index_ns = nanos(start);
    let cold_parsed_files = indexer.stats().parsed_files;
    let memory_after_cold = memory();
    let start = Instant::now();
    let analyzer = ImpactAnalyzer::new(&clean, &index)?;
    let graph_prepare_ns = nanos(start);
    let impact_requests = match reference {
        Some(reference) => requests(&index, reference)?,
        None => {
            let file = index
                .files
                .get(change_path.ok_or("real repository needs change path")?)
                .ok_or("change path is not an indexed file")?;
            vec![ImpactRequest {
                snapshot: index.snapshot,
                generation: index.generation,
                changed: BTreeSet::from([file.file.clone()]),
                transaction_accesses: AccessSet::default(),
                invariants: vec![],
            }]
        }
    };
    let quality = reference
        .map_or(&[][..], |reference| reference.cases.as_slice())
        .iter()
        .zip(&impact_requests)
        .map(|(case, request)| -> Result<Quality> {
            let result = analyzer.analyze(request)?;
            result.validate(&index)?;
            let measured = measure_quality(&result.affected_files, &case.affected_files);
            Ok(Quality {
                case: case.name.clone(),
                changed_element: request
                    .changed
                    .first()
                    .ok_or("empty impact root")?
                    .as_str()
                    .into(),
                predicted_files: result.affected_files.clone(),
                reference_files: case.affected_files.clone(),
                missed_files: case
                    .affected_files
                    .difference(&result.affected_files)
                    .cloned()
                    .collect(),
                extra_files: result
                    .affected_files
                    .difference(&case.affected_files)
                    .cloned()
                    .collect(),
                true_positives: measured.true_positives,
                false_positives: measured.false_positives,
                false_negatives: measured.false_negatives,
                precision: measured.precision,
                recall: measured.recall,
                incomplete: !result.is_complete(),
                rationale: case.rationale.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let query_probe = analyzer.analyze(&impact_requests[0])?;
    query_probe.validate(&index)?;
    let mut cached_impact_query_ns = Vec::with_capacity(queries);
    for query in 0..queries {
        let start = Instant::now();
        let result = analyzer.analyze(&impact_requests[query % impact_requests.len()])?;
        std::hint::black_box(result);
        cached_impact_query_ns.push(nanos(start));
    }
    let memory_after_queries = memory();
    let (dirty, capture_dirty_ns) = capture_changed()?;
    if dirty.snapshot.id() == clean.snapshot.id() {
        return Err("dirty workspace retained clean snapshot identity".into());
    }
    let start = Instant::now();
    let incremental = indexer.update(&dirty.snapshot)?;
    let incremental_index_ns = nanos(start);
    let stats = indexer.stats();
    let mut fresh = Indexer::new(IndexConfig::default())?;
    let start = Instant::now();
    let rebuilt = fresh.build(&dirty.snapshot)?;
    let rebuild_index_ns = nanos(start);
    if incremental != rebuilt {
        return Err("incremental and clean rebuild results differ".into());
    }
    if stats.parsed_files != 1 || stats.reused_files + 1 != cold_parsed_files {
        return Err("unexpected syntax-tree reuse counts".into());
    }
    let memory_after_incremental_and_rebuild = memory();
    let mut source_files = BTreeMap::new();
    for file in index.files.values() {
        if let Some(language) = file.language {
            *source_files.entry(format!("{language:?}")).or_default() += 1;
        }
    }
    Ok(Sample {
        iteration,
        clean_snapshot: clean.snapshot.id().to_string(),
        immutable_git_snapshot: immutable.snapshot.id().to_string(),
        dirty_snapshot: dirty.snapshot.id().to_string(),
        clean_generation: index.generation.to_string(),
        dirty_generation: incremental.generation.to_string(),
        configuration: index.config_digest.to_string(),
        effective_files: index.files.len(),
        source_files,
        effective_bytes: index.files.values().map(|file| file.bytes).sum(),
        clean_capture_manifest_inputs: clean.report.input_count,
        dirty_capture_manifest_inputs: dirty.report.input_count,
        nodes: index.nodes.len(),
        edges: index.edges.len(),
        issues: index.issues.len(),
        changed_nodes: incremental.nodes.len(),
        changed_edges: incremental.edges.len(),
        changed_issues: incremental.issues.len(),
        changed_effective_bytes: incremental.files.values().map(|file| file.bytes).sum(),
        capture_clean_ns,
        capture_repeat_ns,
        capture_git_tree_ns,
        capture_dirty_ns,
        cold_index_ns,
        incremental_index_ns,
        rebuild_index_ns,
        graph_prepare_ns,
        impact_query_complete: query_probe.is_complete(),
        impact_query_affected_files: query_probe.affected_files.len(),
        impact_query_gap_count: query_probe.gaps.len(),
        cached_impact_query_ns,
        cold_parsed_files,
        incremental_parsed_files: stats.parsed_files,
        incremental_reused_files: stats.reused_files,
        rebuild_equivalent: true,
        repeat_capture_equivalent: true,
        live_capture_complete: live.then(|| clean.report.complete()),
        live_capture_census_complete: live.then(|| clean.report.census_complete()),
        immutable_git_capture_complete: immutable.report.complete(),
        memory_before,
        memory_after_cold,
        memory_after_incremental_and_rebuild,
        memory_after_queries,
        quality,
    })
}

fn run_real_iteration(
    iteration: usize,
    queries: usize,
    root: &Path,
    commit: &str,
    change_path: &str,
) -> Result<Sample> {
    let request = CaptureRequest::new(
        root,
        Scope {
            organization: OrganizationId::new("benchmark-org")?,
            workspace: WorkspaceId::new("benchmark-workspace")?,
            repository: RepositoryId::new("actual-repository-benchmark")?,
        },
        Revision(0),
    );
    let memory_before = memory();
    let start = Instant::now();
    let clean = capture_git_tree(&request, commit)?;
    let capture_clean_ns = nanos(start);
    let start = Instant::now();
    let repeated = capture_git_tree(&request, commit)?;
    let capture_repeat_ns = nanos(start);
    if clean != repeated {
        return Err("immutable Git-tree repeat capture changed".into());
    }
    let file = clean
        .effective_files()?
        .remove(change_path)
        .ok_or("change path is not a captured regular file")?;
    if file.kind != celetixo_engine::snapshot::InputKind::File {
        return Err("change path must be a regular source file".into());
    }
    let language = Language::for_path(change_path)
        .ok_or("overlay change needs a supported source language")?;
    let mut overlay_bytes = file.bytes;
    overlay_bytes.extend_from_slice(if language == Language::Python {
        b"\n# celetixo benchmark overlay-only edit\n"
    } else {
        b"\n// celetixo benchmark overlay-only edit\n"
    });
    let mut changed_request = request.clone();
    changed_request.overlays.insert(
        change_path.into(),
        Overlay {
            version: 1,
            bytes: Some(overlay_bytes),
            mode: file.mode,
        },
    );
    let prepared = PreparedCapture {
        immutable: clean.clone(),
        clean,
        capture_clean_ns,
        capture_repeat_ns,
        capture_git_tree_ns: capture_clean_ns,
        memory_before,
        live: false,
    };
    measure_pair(
        iteration,
        queries,
        prepared,
        None,
        Some(change_path),
        || {
            let start = Instant::now();
            let changed = capture_git_tree(&changed_request, commit)?;
            Ok((changed, nanos(start)))
        },
    )
}

fn option(args: &[String], name: &str, default: usize, max: usize) -> Result<usize> {
    let value = match args.iter().position(|arg| arg == name) {
        Some(position) => args
            .get(position + 1)
            .ok_or_else(|| format!("missing {name} value"))?
            .parse()?,
        None => default,
    };
    if value == 0 || value > max {
        return Err(format!("{name} must be in 1..={max}").into());
    }
    Ok(value)
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "repository_bench [--copies 1..100] [--iterations 1..20] [--queries 1..10000]\nrepository_bench --repository PATH --commit FULL_OID --change-path RELATIVE_SOURCE_PATH [--iterations N] [--queries N]\nGenerated mode writes only a disposable fixture. Repository mode reads an immutable Git tree and applies an in-memory overlay; it never edits that repository. Both emit JSON. Intended for Linux. Cold means an empty Indexer cache, not a flushed operating-system cache."
        );
        return Ok(());
    }
    if args.len() % 2 != 0
        || args.chunks(2).any(|pair| {
            !matches!(
                pair[0].as_str(),
                "--copies"
                    | "--iterations"
                    | "--queries"
                    | "--repository"
                    | "--commit"
                    | "--change-path"
            )
        })
        || args
            .chunks(2)
            .map(|pair| &pair[0])
            .collect::<BTreeSet<_>>()
            .len()
            != args.len() / 2
    {
        return Err("unknown or malformed option; use --help".into());
    }
    let iterations = option(&args, "--iterations", 3, 20)?;
    let queries = option(&args, "--queries", 100, 10_000)?;
    let scratch = tempfile::tempdir()?;
    let git_version = git(scratch.path(), &["--version"])?;
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .map(|position| args[position + 1].as_str())
    };
    let capture_filesystem = filesystem_type(
        value("--repository")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .as_path(),
    );
    let (corpus, samples) = if let Some(repository) = value("--repository") {
        if value("--copies").is_some() {
            return Err("--copies applies only to generated fixtures".into());
        }
        let commit =
            value("--commit").ok_or("repository mode requires an explicit full --commit OID")?;
        let change_path = value("--change-path").ok_or("repository mode requires --change-path")?;
        let mut samples = Vec::new();
        for iteration in 0..iterations {
            samples.push(run_real_iteration(
                iteration,
                queries,
                Path::new(repository),
                commit,
                change_path,
            )?);
        }
        (
            Corpus {
                fixture: "celetixo-actual-repository/v1",
                reference: None,
                reference_unit: None,
                copies: None,
                repository_commit: Some(commit.into()),
                change_path: Some(change_path.into()),
                capture_mode: "immutable-git-tree-and-overlay",
                fixture_content_digest: None,
                manual_reference_digest: None,
            },
            samples,
        )
    } else {
        if value("--commit").is_some() || value("--change-path").is_some() {
            return Err("--commit and --change-path require --repository".into());
        }
        let copies = option(&args, "--copies", 1, 100)?;
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/benchmark");
        let reference_bytes = fs::read(fixture.join("reference.json"))?;
        let reference: Reference = serde_json::from_slice(&reference_bytes)?;
        if reference.cases.is_empty() {
            return Err("manual reference has no cases".into());
        }
        let mut templates = BTreeMap::new();
        fixture_inputs(
            &fixture.join("template"),
            &fixture.join("template"),
            &mut templates,
        )?;
        let fixture_content_digest = celetixo_engine::ids::Digest::of(
            "celetixo/benchmark-template/v1",
            &serde_json::to_vec(&templates)?,
        )
        .to_string();
        let mut samples = Vec::new();
        for iteration in 0..iterations {
            samples.push(run_iteration(
                iteration, copies, queries, &templates, &reference,
            )?);
        }
        (
            Corpus {
                fixture: "celetixo-mixed-language-fixture/v1",
                reference: Some(reference.version),
                reference_unit: Some(reference.unit),
                copies: Some(copies),
                repository_commit: None,
                change_path: None,
                capture_mode: "live-git-workspace-and-dirty-edits",
                fixture_content_digest: Some(fixture_content_digest),
                manual_reference_digest: Some(
                    celetixo_engine::ids::Digest::of(
                        "celetixo/benchmark-manual-reference/v1",
                        &reference_bytes,
                    )
                    .to_string(),
                ),
            },
            samples,
        )
    };
    let parsers = [
        Language::TypeScript,
        Language::JavaScript,
        Language::Python,
        Language::Go,
        Language::Rust,
    ]
    .into_iter()
    .map(|language| language.parser_version().to_owned())
    .collect();
    let cpu = fs::read_to_string("/proc/cpuinfo").ok().and_then(|text| {
        text.lines().find_map(|line| {
            line.strip_prefix("model name")
                .and_then(|value| value.strip_prefix("\t: "))
                .map(str::to_owned)
        })
    });
    if samples.iter().skip(1).any(|sample| {
        sample.clean_snapshot != samples[0].clean_snapshot
            || sample.dirty_snapshot != samples[0].dirty_snapshot
            || sample.clean_generation != samples[0].clean_generation
            || sample.dirty_generation != samples[0].dirty_generation
    }) {
        return Err(
            "repeated benchmark inputs produced different snapshot/index identities".into(),
        );
    }
    let report = Report {
        schema: "celetixo/repository-benchmark/v1",
        corpus,
        iterations,
        queries_per_iteration: queries,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        native_cflags_at_build: option_env!("CFLAGS"),
        crate_cc_no_defaults_at_build: option_env!("CRATE_CC_NO_DEFAULTS"),
        rustc_version_recorded_at_build: option_env!("CELETIXO_BENCH_RUSTC_VERSION"),
        native_compiler_recorded_at_build: option_env!("CELETIXO_BENCH_CC_VERSION"),
        rustup_toolchain_at_build: option_env!("RUSTUP_TOOLCHAIN"),
        analyzer: IndexConfig::default().analyzer_version,
        parsers,
        measurement_implementation_digest: implementation_digest(),
        git_version,
        operating_system: fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_else(|_| std::env::consts::OS.into())
            .trim()
            .into(),
        architecture: std::env::consts::ARCH,
        cpu,
        capture_filesystem,
        samples,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

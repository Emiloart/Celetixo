#!/usr/bin/env python3
"""Check artifact equivalence before comparing baseline/optimized measurements."""
import argparse
import json
import statistics
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    corpus_fields = (
        "schema", "fixture", "reference", "reference_unit", "copies",
        "repository_commit", "change_path", "capture_mode", "iterations",
        "queries_per_iteration", "build_profile", "native_cflags_at_build",
        "crate_cc_no_defaults_at_build", "rustc_version_recorded_at_build",
        "native_compiler_recorded_at_build", "rustup_toolchain_at_build",
        "analyzer", "parsers", "fixture_content_digest", "manual_reference_digest",
        "git_version", "operating_system", "architecture", "cpu", "capture_filesystem",
    )
    artifact_fields = (
        "clean_snapshot", "immutable_git_snapshot", "dirty_snapshot",
        "clean_generation", "dirty_generation", "configuration", "effective_files",
        "source_files", "effective_bytes", "clean_capture_manifest_inputs",
        "dirty_capture_manifest_inputs", "nodes", "edges", "issues",
        "changed_nodes", "changed_edges", "changed_issues", "changed_effective_bytes",
        "impact_query_complete", "impact_query_affected_files", "impact_query_gap_count",
        "cold_parsed_files", "incremental_parsed_files", "incremental_reused_files",
        "rebuild_equivalent", "repeat_capture_equivalent", "live_capture_complete",
        "live_capture_census_complete", "immutable_git_capture_complete", "quality",
    )
    print("| Corpus | Artifact equality | Baseline / optimized cold median ms | Baseline / optimized incremental median ms |")
    print("|---|---|---:|---:|")
    for corpus in ("copies-1", "copies-5", "copies-20", "actual-celetixo"):
        before = json.loads((args.directory / f"goal-02-baseline-{corpus}.json").read_text())
        after = json.loads((args.directory / f"goal-02-optimized-{corpus}.json").read_text())
        for field in corpus_fields:
            require(before[field] == after[field], f"{corpus}: different input/build environment field {field}")
        require(len(before["samples"]) == len(after["samples"]) == before["iterations"], f"{corpus}: different sample counts")
        for old, new in zip(before["samples"], after["samples"]):
            for field in artifact_fields:
                require(old[field] == new[field], f"{corpus}: different artifact field {field}")
        median = lambda report, field: statistics.median(s[field] for s in report["samples"]) / 1e6
        print(f'| {corpus} | Exact recorded identities/graph/quality | {median(before, "cold_index_ns"):.3f} / {median(after, "cold_index_ns"):.3f} | {median(before, "incremental_index_ns"):.3f} / {median(after, "incremental_index_ns"):.3f} |')
    print("\nSnapshot/index-generation digests bind complete artifacts, while raw reports retain separate implementation fingerprints. This comparison does not claim performance significance from three observations.")


if __name__ == "__main__":
    main()

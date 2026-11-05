#!/usr/bin/env python3
"""Summarize checked-in repository_bench JSON without discarding slow samples."""
import argparse
import json
import math
import statistics
from pathlib import Path


def percentile(values, fraction):
    """Nearest-rank percentile; every timing remains available in raw JSON."""
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--targets", action="store_true", help="also derive rounded same-machine investigation ceilings from every observed maximum")
    parser.add_argument("reports", nargs="+", type=Path)
    args = parser.parse_args()
    print("| Corpus / copies | Files / bytes | Capture clean median [min, max] ms | Capture changed median ms | Git-tree capture median ms | Cold index median ms | Incremental median ms | Rebuild median ms | Query p50 / p95 ms | Process peak MiB |")
    print("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    targets = []
    for path in args.reports:
        report = json.loads(path.read_text())
        require(report["schema"] == "celetixo/repository-benchmark/v1", f"{path}: unsupported report schema")
        samples = report["samples"]
        require(len(samples) == report["iterations"] and samples, f"{path}: missing iterations")
        require(all(s["rebuild_equivalent"] and s["repeat_capture_equivalent"] for s in samples), f"{path}: capture/rebuild equivalence failed")
        require(all(s["immutable_git_capture_complete"] and not s["impact_query_complete"] for s in samples), f"{path}: incorrect capture/impact completeness")
        if report["copies"] is not None:
            require(all(s["live_capture_complete"] is False and s["live_capture_census_complete"] for s in samples), f"{path}: live capture uncertainty missing")
        else:
            require(all(s["live_capture_complete"] is None and s["live_capture_census_complete"] is None and not s["quality"] for s in samples), f"{path}: actual corpus must not claim live capture or labeled quality")
        for sample in samples:
            require(sample["incremental_parsed_files"] == 1, f"{path}: expected one changed syntax tree")
            require(sample["incremental_reused_files"] == sample["cold_parsed_files"] - 1, f"{path}: incorrect unchanged-tree reuse")
            require(len(sample["cached_impact_query_ns"]) == report["queries_per_iteration"], f"{path}: missing query samples")
            for quality in sample["quality"]:
                require(quality["incomplete"], f"{path}: structural quality cannot certify completeness")
                require(quality["false_positives"] == 0, f"{path}: unexpected reference false positive")
                require(quality["false_negatives"] == (1 if quality["case"] == "python-base-with-dynamic-consumer" else 0), f"{path}: unexpected reference false negative")
        median = lambda field: statistics.median(s[field] for s in samples) / 1e6
        clean = [s["capture_clean_ns"] / 1e6 for s in samples]
        queries = [timing / 1e6 for s in samples for timing in s["cached_impact_query_ns"]]
        peaks = [s["memory_after_incremental_and_rebuild"]["lifetime_peak_rss_kib"] / 1024 for s in samples]
        first = samples[0]
        label = report["copies"] if report["copies"] is not None else "Actual repository"
        print(f'| {label} | {first["effective_files"]} / {first["effective_bytes"]} | {median("capture_clean_ns"):.3f} [{min(clean):.3f}, {max(clean):.3f}] | {median("capture_dirty_ns"):.3f} | {median("capture_git_tree_ns"):.3f} | {median("cold_index_ns"):.3f} | {median("incremental_index_ns"):.3f} | {median("rebuild_index_ns"):.3f} | {percentile(queries, .50):.3f} / {percentile(queries, .95):.3f} | {max(peaks):.2f} |')
        round_ms = lambda field: math.ceil(max(s[field] for s in samples) / 100_000_000) * 100
        capture_max = max(s[field] for s in samples for field in ("capture_clean_ns", "capture_repeat_ns", "capture_git_tree_ns", "capture_dirty_ns"))
        targets.append((label, math.ceil(capture_max / 100_000_000) * 100, round_ms("cold_index_ns"), round_ms("incremental_index_ns"), round_ms("rebuild_index_ns"), math.ceil(max(queries)), math.ceil(max(peaks))))
    print("\nTimes are observed wall times. Query percentiles pool all recorded queries at each scale; no outliers are removed.")
    if args.targets:
        print("\n| Corpus / copies | Capture ceiling ms | Cold index ceiling ms | Incremental ceiling ms | Rebuild ceiling ms | Single-query ceiling ms | Process peak ceiling MiB |")
        print("|---:|---:|---:|---:|---:|---:|---:|")
        for row in targets:
            print("| " + " | ".join(map(str, row)) + " |")
        print("\nInvestigation ceilings round every observed phase maximum upward to 100ms, every query maximum upward to 1ms, and peak RSS upward to 1MiB. These are local regression targets, not production SLOs or correctness gates.")


if __name__ == "__main__":
    main()

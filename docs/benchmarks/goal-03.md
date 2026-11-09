# Goal 03 measured baseline

Measured executable source: [`fa888ed49cda2f151f64491986aa8ea9d7afbb36`](https://github.com/Emiloart/Celetixo/commit/fa888ed49cda2f151f64491986aa8ea9d7afbb36). The source and working tree were clean before and after collection. The run used evidence v2 and Engine-scoped executor authentication; no hardware or remote-worker attestation is claimed.

Recorded 2026-11-09T11:10:44.827080+00:00 through 2026-11-09T11:12:44.496796+00:00. Each case has **three observed repetitions**. These are local reference-component measurements, with no cold-cache or production-capacity claim.

## Reproduction and raw evidence

Follow the [protocol](goal-03-protocol.md). [Exact commands, source/tool/binary hashes and environment](goal-03/run.json), [command output](goal-03/commands.log), [filesystem context](goal-03/context.json), [Celetixo T0 samples](goal-03/celetixo-t0.json), [xxhash T0 samples](goal-03/xxhash-t0.json), [generated T1 and recovery samples](goal-03/generated-t1.json), and [derived statistics/targets](goal-03/summary.json) are preserved. The [external-source provision record](goal-03/xxhash-provision.json) pins every tracked file.

Recompute the summary without network or new dependencies:

```sh
python3 docs/benchmarks/goal-03/summarize.py docs/benchmarks/goal-03
```

The recomputed summary was byte-identical. Rust was built in release mode with the recorded toolchain and `CARGO_INCREMENTAL=0 CRATE_CC_NO_DEFAULTS=1 CFLAGS="-target x86_64-linux-gnu -fPIC -O3"`. Compiler invocations and any reused artifacts are visible in the command log. Tool origin/signature records are retained in the [execution toolchain provenance](../implementation/goal-03-execution-validation/toolchain.json); the run records the actual file hashes again.

## Environment and interpretation

- CPU: model name	: Intel(R) Core(TM) i7-7600U CPU @ 2.80GHz
- Kernel: `6.18.54.1-microsoft-standard-WSL2`; architecture: `x86_64`.
- Compiler: `rustc 1.94.1 (e408947bf 2026-03-25)`; runtime: rootless runc 1.5.2; check: Go 1.26.2 gofmt.
- Celetixo inputs are on `/mnt/c` (9p); xxhash inputs, disposable copies and registered workspaces are on the Linux ext4 filesystem. Capture results therefore do not isolate repository size from filesystem placement.
- T1 generated inputs are fully enumerated reference cases. Their actual structural analysis remains incomplete; the separately labelled finite reference model admits only these known checks.
- Host RSS/high water are `/proc/self/status` observations for the benchmark process. Guest peak/cpu counters are backend observations. Neither is exact per-phase allocation attribution.
- Per-phase timing probes can overlap. Materialization includes independent patch replay/census; T1 total excludes fixture construction. Do not sum independent probes.

## Actual repository capture and pure T0 operations

Capture uses the exact selected commit, without executing repository code or installing dependencies. No complete semantic impact or Engine execution admission is claimed for these actual repositories. xxhash is pinned to `998dce232f17418a7a5721ecf87ca714025a3243`.

| Input | Files | Content bytes | Capture median / max ms |
|---|---:|---:|---:|
| celetixo | 163 | 2352661 | 24325.843 / 25541.804 |
| xxhash | 24 | 36958 | 152.060 / 154.673 |

All operation timings below are milliseconds, **median / observed maximum**. Every materialization reproduced the exact candidate and removed its private copy. Content reuse was zero.

| Input | Case | Apply | Manifest hash | Copy + replay/census | Copied bytes |
|---|---|---:|---:|---:|---:|
| celetixo | replace-original | 117.630 / 122.335 | 14.891 / 15.203 | 317.275 / 328.537 | 2352662 |
| celetixo | delete-original | 112.585 / 117.022 | 12.266 / 14.721 | 305.969 / 310.670 | 2350995 |
| celetixo | rename-original | 121.053 / 137.703 | 14.183 / 16.296 | 322.234 / 358.461 | 2352661 |
| celetixo | mode-original | 121.328 / 123.119 | 13.386 / 15.560 | 319.169 / 327.943 | 2352661 |
| celetixo | create-4KiB | 109.591 / 120.983 | 12.656 / 12.659 | 301.809 / 314.457 | 2356757 |
| celetixo | create-64KiB | 112.263 / 119.198 | 14.395 / 14.985 | 335.160 / 376.230 | 2418197 |
| celetixo | create-1MiB | 127.088 / 131.250 | 19.773 / 20.614 | 428.601 / 503.483 | 3401237 |
| xxhash | replace-original | 2.206 / 2.352 | 0.329 / 0.335 | 9.559 / 10.532 | 36959 |
| xxhash | delete-original | 1.973 / 2.019 | 0.245 / 0.301 | 8.452 / 8.826 | 35653 |
| xxhash | rename-original | 1.863 / 2.384 | 0.202 / 0.238 | 8.206 / 9.712 | 36958 |
| xxhash | mode-original | 1.695 / 2.169 | 0.334 / 0.378 | 9.158 / 10.780 | 36958 |
| xxhash | create-4KiB | 1.966 / 2.685 | 0.236 / 0.370 | 10.316 / 14.104 | 41054 |
| xxhash | create-64KiB | 3.280 / 3.526 | 0.565 / 0.639 | 11.372 / 12.221 | 102494 |
| xxhash | create-1MiB | 20.333 / 20.430 | 7.470 / 7.744 | 53.323 / 54.276 | 1085534 |

Canonical encoding/decoding, replay validation, extra census, cleanup, logical copy throughput and host memory are retained for every sample in the raw files and summary. The copy-throughput number includes validation cost and is not physical disk bandwidth.

## Real isolated checks and Engine evidence admission

Each generated case ran the configured syntax check in the reviewed T1 boundary, persisted canonical evidence, verified artifact retrieval and obtained the Engine receipt. `gofmt -l` exit zero is a syntax outcome, not a format-conformance, compiler or Go test result.

| Generated case | Files | Materialization ms | T1 run total ms | Evidence publication ms | Engine gate ms |
|---|---:|---:|---:|---:|---:|
| 4KiB-source | 3 | 24.109 / 36.860 | 631.912 / 862.649 | 212.276 / 219.407 | 9.562 / 11.500 |
| 64KiB-source | 3 | 144.945 / 145.639 | 1003.589 / 1031.442 | 367.186 / 367.501 | 23.668 / 25.090 |
| 64KiB-with-128-assets | 131 | 3378.464 / 3378.517 | 4406.499 / 4410.427 | 435.067 / 455.790 | 31.970 / 36.370 |

The 131-file case spends most of this reference run in validated materialization. That is an observed optimization candidate, not a reason to omit ownership checks, replay or source census.

| Generated case | Backend preparation ms | Startup ms | Command supervision ms | Runtime cleanup ms |
|---|---:|---:|---:|---:|
| 4KiB-source | 105.090 / 321.553 | 96.652 / 104.423 | 6.474 / 8.655 | 130.909 / 133.267 |
| 64KiB-source | 108.875 / 109.075 | 123.857 / 127.132 | 15.926 / 22.006 | 135.632 / 137.401 |
| 64KiB-with-128-assets | 132.200 / 174.786 | 121.508 / 133.386 | 22.652 / 24.966 | 135.429 / 140.097 |

| Generated case | Guest observed wall ms | Guest CPU ms | Guest observed peak MiB | Host process lifetime high-water MiB |
|---|---:|---:|---:|---:|
| 4KiB-source | 105.000 / 110.000 | 36.229 / 38.947 | 3.23 / 3.46 | 32.14 / 32.14 |
| 64KiB-source | 142.000 / 144.000 | 36.933 / 37.567 | 3.02 / 3.21 | 32.14 / 32.14 |
| 64KiB-with-128-assets | 144.000 / 158.000 | 37.062 / 38.801 | 3.20 / 3.46 | 32.14 / 32.14 |

## Capacity, recovery and cache boundary

| Probe | Median / max ms |
|---|---:|
| busy_rejection_us | 18.850 / 19.552 |
| preparation_and_controlled_failure_us | 126.165 / 129.636 |
| owned_orphan_recovery_us | 85.534 / 91.184 |
| idempotent_repeated_recovery_us | 0.466 / 0.813 |

The capacity probe genuinely overlaps two admitted attempts while one holds the single backend slot. The contender is rejected as Busy: no queue delay or parallel execution throughput is inferred. The recovery probe interrupts the controller after durable claim publication, before process launch; owned source/runtime resources are removed, repeated recovery is idempotent, and no result is invented. Separate actual SIGKILL integration tests prove post-launch deadline/process-tree behavior.

Cache reuse is disabled. Each sample records complete cache-key construction and validation cost, plus `cache_reuse_enabled=false`; no reusable-hit or distributed-cache lookup latency exists to report.

## Initial tracking targets

The summary publishes a provisional timing ceiling for every measured case/operation, set to that case’s **observed maximum of three samples**. The throughput tracking floor is its observed minimum. Keep future matching-case medians at or below the recorded medians and review observations beyond these ceilings with additional measurements. These are local regression-tracking targets, not statistically established SLOs or evidence of production capacity. Do not compare mismatched source/toolchain/backend/filesystem cases.

| Scope | Initial measured tracking ceiling |
|---|---:|
| celetixo capture | 25541.804 ms |
| xxhash capture | 154.673 ms |
| 4KiB-source T1 run | 862.649 ms |
| 64KiB-source T1 run | 1031.442 ms |
| 64KiB-with-128-assets T1 run | 4410.427 ms |

A safety violation invalidates a run regardless of speed. Any optimization must preserve exact identity, authorization, resource policy, cleanup and independent Engine evidence checks. Stronger tiers, broader language harnesses, persistent distributed authority, automatic dependency acquisition, source publication and public production readiness remain unimplemented boundaries, not implied benchmark capabilities.

# Local Linux T1 Operations Runbook

**Document class:** Operational procedure for a bounded validated profile  
**Reviewed against:** `main` at `456bb375108532d9503c82fa8f6e7acfdfedddaf` (2026-11-11)  
**Implementation authority:** Goal 03 execution component and Goal 03 acceptance record  
**Scope:** Local Linux x86_64 development/reproduction only. This is not a production deployment guide for the distributed target architecture.

## 1. Operating boundary

The currently validated execution profile is:

- Linux x86_64.
- Rootless `runc` 1.5.2.
- Delegated cgroup v2 with a live systemd user manager and the validated user-service cgroup layout.
- Reviewed, content-pinned runtime, entrypoint, formatter, interpreter and libraries.
- A private tooling directory supplied through `CELETIXO_OCI_HOME`.
- The Engine reference implementation and local source-artifact store.

The runner refuses host UID 0 and fails closed when required delegation, user-manager or cgroup properties cannot be established. It has no host-command fallback.

**What this profile is:** an actual Linux T1 execution path with private source materialization, runtime isolation observations, resource controls, persisted local artifacts and Engine evidence admission.

**What this profile is not:** a production service, distributed worker pool, remote execution service, public multi-tenant deployment, durable distributed transaction authority, microVM, or full language harness. The Engine's authoritative transaction state remains in memory. T2/T3, network allowlists, credential delivery and custom output handoff are unsupported.

Read [ADR-0004](../../adr/0004-internal-first-public-capable-architecture.md), the [threat model](../../spec/threat-model.md) and the [Goal 03 acceptance record](../goal-03-acceptance.md) before using this profile with non-disposable data.

## 2. Preconditions

### Host requirements

1. Linux x86_64 with cgroup v2.
2. A non-root user session with a live systemd user manager.
3. Delegated cgroup v2 controllers and the exact user-service cgroup layout exercised by the validation.
4. Rootless execution support for the reviewed runc profile.
5. Sufficient disk space for private workspaces, journals, artifacts and logs.
6. No expectation that the host kernel or host administrator is isolated from the workload.

Do not work around a failed preflight by running as root, disabling a control, changing the cgroup layout, or using host execution. A changed host/runtime profile requires a new validation run.

### Tooling directory

The reviewed private directory contains:

```text
<CELETIXO_OCI_HOME>/
  runc-1.5.2
  celetixo-exec-init
  toolchain/
    gofmt
    lib/
      libc.so.6
      ld-linux-x86-64.so.2
```

The fixture also pins the installed `/usr/bin/busctl` control-plane tool. The measured profile records:

| Component | Recorded identity |
|---|---|
| runc | 1.5.2; SHA-256 `599f6f94ff8c5057241eff0d54c3c74f95c34935b6457b33fe545defc61e9488` |
| gofmt | Go 1.26.2; SHA-256 `0802a3784f7b2a924ccde5b34f6727ffe74e0193a1c0eea4cd773863fce4528a` |
| Entry point | Built from `crates/execution/src/bin/celetixo-exec-init.rs`; resulting ELF interpreter and required libraries must be checked |
| Control-plane tool | Installed `/usr/bin/busctl`; exact identity belongs in the run record |

These hashes identify the reviewed files, not their upstream authenticity. Verify origin/signature and retain compiler/library provenance. Do not download or install runtime tools from a workload or repository under test.

## 3. Preflight and baseline checks

Run from a clean checkout on the intended host:

```sh
id
uname -a
stat -fc %T /sys/fs/cgroup
systemctl --user is-system-running
command -v busctl
rustc --version
cargo --version
git status --short --branch
```

Confirm cgroup v2, the non-root execution identity, active user manager, expected Rust toolchain and clean/understood working tree. The commands above are observations, not a substitute for the runner's actual effective-control checks.

Run portable workspace checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --examples --locked
```

These validate source/build behavior. They do not prove physical OCI containment.

## 4. Actual T1 integration validation

Supply the reviewed private tooling directory. The runner does not download dependencies or silently substitute another runtime.

```sh
CELETIXO_OCI_HOME=/reviewed/private/tooling \
  cargo test --locked -p celetixo-execution \
  --test oci --test source_flow -- --ignored --test-threads=1
```

Run the explicit OCI tests serially. They exercise actual isolation/resource boundaries, source immutability, cancellation, timeout, controller loss, owned cleanup, publication faults, PID publication, mismatched-base rejection, fresh retry with preserved failed history and patch-to-Engine evidence integration.

The benchmark reproduction is:

```sh
CELETIXO_OCI_HOME=/reviewed/private/tooling \
  cargo run --release --locked -p celetixo-execution \
  --example source_execution_bench -- --iterations 3
```

Retain the raw output and exact source commit, input identities, runtime/tool hashes, compiler, kernel, CPU, cgroup profile and commands. The published baseline disables cache reuse. It does not establish cold-build performance, production throughput, queue latency or a universal SLO.

See [Goal 03 measurement protocol](../benchmarks/goal-03-protocol.md) and [measured baseline](../benchmarks/goal-03.md).

## 5. Workload and credential policy

- Treat repository source, patches, dependencies, manifests, tool output and test output as untrusted.
- Capture/index operations must not execute repository hooks, filters, scripts or source.
- Do not place production credentials in snapshots, prompts, task payloads, environment variables or sandbox files.
- The validated runner does not implement scoped credential delivery. Do not work around this by mounting host credentials.
- T1 network access is not a general network-allowlist feature. T2/T3, credential delivery and custom output handoff are unsupported and must fail closed.
- Do not increase resource limits or alter seccomp, namespaces, mounts, capabilities or no-new-privileges settings without reviewing and revalidating the resulting profile.
- Do not run untrusted checks on the host because T1 prerequisites are unavailable.

## 6. Persistent artifacts, backup and restore

The implementation provides `LocalSourceArtifactStore::create/open` with synchronized store metadata, content-addressed objects, bounded private files, publication synchronization and verified readback. `temporary` is a test adapter and removes files on final drop; it is not an operational store.

The local store is not a distributed object store, retention daemon, backup service or protection against a host administrator. The Engine transaction store is in memory. Therefore:

- Persisted artifacts may survive process restart when using the persistent store.
- In-memory transaction/lease/authorization state is not restored by copying the artifact directory.
- Restoring artifacts does not recreate lost Engine authority or permit old evidence to be admitted under a different evidence/runtime generation.
- Do not represent artifact backup as a complete platform backup.

### Backup procedure

There is no built-in production backup command or coordinated backup service in this profile. Until one exists:

1. Stop admitting new work.
2. Ensure no execution is active and all runtime termination/cleanup states are known.
3. Close the application/store cleanly.
4. Take a filesystem-level backup of the entire persistent store root, including its metadata and content-addressed objects, preserving permissions and directory structure.
5. Record the backup timestamp, source revision, store identity, filesystem method and integrity manifest/digests.
6. Keep the backup access-controlled and encrypted according to the data classification policy.
7. Do not copy a live store while writers are active and call the result consistent.

### Restore procedure

1. Restore the complete store root to a private location; do not merge it into an existing store.
2. Restrict access to the operator and service identity.
3. Reopen it using the version-matched `LocalSourceArtifactStore::open` path.
4. Require the store's own validation/readback and pending-object reconciliation to succeed.
5. Investigate missing, mismatched or orphaned objects; do not fabricate records or mark them admitted.
6. Start a fresh Engine state and re-admit work through current authorization, candidate and evidence checks. Do not assume that old in-memory transaction authority has been restored.
7. Record the restore test and unresolved gaps.

This is a local artifact-store recovery procedure, not disaster recovery for a distributed Celetixo deployment. A coordinated, automated backup/restore system remains a target requirement.

## 7. Upgrade and runtime-generation changes

Treat changes to any of the following as a new execution profile: runc, entrypoint, formatter/interpreter, libraries, host kernel, cgroup layout, systemd control behavior, namespace/seccomp policy, materializer, artifact format or backend adapter.

Before upgrading:

1. Stop new admissions and drain or explicitly terminate active work.
2. Resolve every owned runtime journal. Unknown termination or cleanup must remain quarantined.
3. Back up the persistent artifact store using the procedure above.
4. Record old and proposed tool hashes, source revisions and host profile.
5. Run portable tests, actual OCI integration tests and the benchmark protocol on the candidate profile.
6. Review the complete diff and the updated threat model/capability matrix.
7. Reopen and validate retained artifacts using the supported version path.

Evidence binds the backend/runtime/image/control generation. Do not reinterpret an old journal under a new generation, relabel old evidence, or assume an old v1 envelope is admissible under the v2 gate. Retained historical bytes are not automatically current authorization.

Rollback means returning to the exact previously validated source and tooling generation with its matching state. It does not mean weakening checks or reusing incompatible evidence.

## 8. Observability and operational records

The current profile exposes local test output, structured errors, persisted check records, artifacts, runtime ownership/journal state and benchmark records. It does not provide the full distributed observability target: centralized metrics/traces, service dashboards, alerting, on-call routing, fleet health or tenant-facing audit console.

For every actual run, preserve:
- source commit and working-tree identity;
- snapshot, transaction, attempt, candidate and plan identities;
- policy/backend/runtime/image generation;
- exact toolchain and host profile;
- command, start/end times and exit status;
- effective isolation observations and resource counters;
- output/artifact digests and persistence result;
- cancellation, timeout, retry and cleanup outcomes;
- any missing observations or uncertainty.

A missing resource sample is not an exact lifetime measurement. A successful check is not proof of semantic completeness. A cleanup marker is not a check result.

## 9. Incident response and recovery

### Suspected escape, unauthorized access or secret exposure

1. Stop new admissions.
2. Use the existing scoped control path to terminate affected execution; do not signal arbitrary PIDs by guess.
3. Preserve runtime journals, ownership records, logs, artifacts, source/tool hashes and host observations.
4. If termination or cleanup is uncertain, retain/quarantine the workspace. Do not delete it based on a path string or empty lock.
5. Revoke affected credentials at their source. The runner does not provide a credential-management service.
6. Restrict access to evidence that may contain source, secrets or exploit payloads.
7. Determine affected snapshots, transactions, artifacts and data recipients.
8. Record the incident, containment actions, recovery decision and regression test.
9. Revalidate the host/runtime profile before resuming execution.

### Lost controller or interrupted execution

Use only the exact cleanup-only authority and ownership journal for the affected registry identity. Cleanup authority can query/reconcile the owned runtime; it cannot resume a lost transaction, rerun checks or recreate evidence authority. If ownership records are malformed or damaged, quarantine for operator review. Do not infer deletion authority.

### Failed or stale evidence

Do not manually edit an envelope, artifact, check result or journal to make it pass. Preserve the failed attempt and create a fresh authorized attempt against the current snapshot and policy. The Engine must independently validate the persisted objects and current authority.

## 10. Distributed architecture: target, not this runbook

The accepted architecture includes PostgreSQL authoritative state, NATS JetStream event distribution, Temporal durable workflows, ClickHouse analytics, S3-compatible immutable evidence storage, provider-neutral Brain services and remote execution pools. These are target architecture responsibilities from ADR-0001/0002, not services deployed by the validated local T1 profile.

Production operations still require implementation and validation of:
- service identity, authentication and authorization;
- durable commit linearization and multi-instance concurrency;
- outbox/reconciliation and event/workflow recovery;
- managed secrets and provider credential lifecycle;
- tenant/workspace isolation across every service and backup;
- centralized observability, alerting and audit;
- coordinated backup, restore and disaster recovery;
- migrations, compatibility and rollback;
- incident response and security operations.

Do not use this local runbook as evidence that those distributed controls exist.

## 11. Operational readiness statement

The validated profile is suitable for controlled local engineering and reproduction on a matching host/tooling configuration. It is not a production deployment approval. Any change to host, runtime, toolchain, isolation profile or evidence generation requires fresh validation. Public release and distributed production operation require a separate readiness review under ADR-0004.

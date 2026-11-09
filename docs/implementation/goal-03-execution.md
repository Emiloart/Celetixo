# Goal 03 execution component

This component implements private materialization, a concrete Linux T1 runner,
local immutable artifacts and their integration with the Engine evidence gate.
Goal 03 remains in progress. Representative measurements, final clean-checkout
validation and the complete acceptance audit remain required before its exit gate
can be claimed.

## Authority and data flow

The trusted embedding supplies an Engine admission and a separately granted
`SourceExecutorCapability`. A transaction session, principal ID, patch producer
string or successful process result cannot substitute for that capability.

`VerificationBackend` returns common per-check execution facts, output bytes and
optional actual isolation observations. OCI projection lives in its adapter;
the coordinator does not depend on an OCI record. The sealed interface permits
reviewed implementations. A missing observation stays missing and cannot become
a passing placeholder. Binding checks and required-artifact checks remain
independent Engine responsibilities.

`SourceRunCoordinator` claims once, independently materializes the exact Engine
candidate, runs the policy plan, confirms source integrity and cleanup, persists
provenance and check artifacts, and delivers their canonical envelope. The Engine
reads and validates those objects through an administrator-registered reader. It
rechecks current authority after I/O and derives its own assessment. Passing
source evidence moves a transaction to Validating; source publication remains
unsupported.

The in-memory Engine uses an explicit development clock. A trusted embedding
must retain one `MonotonicEngineClock` per store, starting with that store and
covering idle time as well as checks. Ordinary workers cannot drive the clock or
hold the administrator. This is not a durable production lease service.

## Materialization

`PrivateWorkspace` applies and independently replays a versioned patch using
immutable snapshot bytes. It creates private files with directory-relative,
no-follow operations, reproduces regular/executable modes and permitted internal
symlinks, and checks an exact directory/content/mode census. It does not read or
change a live source checkout. Copying files is a T0 operation, not isolation.

Non-ASCII paths, opaque Gitlinks and unsupported captured Unix-directory metadata
fail explicitly. Git administration retained as snapshot provenance is not
installed as an execution Git directory. Source inputs are mounted read-only by
the actual runner. Separate workspaces do not share writable source files.

An execution-retention latch preserves source while process termination or
runtime cleanup is unknown. Confirmed runtime cleanup permits ordinary workspace
cleanup. A failed identity check, failed deletion or unknown termination retains
an explicit quarantine hint; a path string alone never authorizes deletion.

Execution copies now require `WorkspaceRegistry::create/open`, with exact scope,
root-inode identity and explicit root/count/input-byte quotas. Ownership metadata
is published and synchronized before copying. The registry does not disappear
when its last in-process handle drops. Plain `PrivateWorkspace` copies remain
T0-only; the actual backend refuses them.

A separate live-owner lock prevents orphan recovery from deleting an active
copy. A durable runtime-retention latch independently prevents deletion while
termination is unknown. Recovery requires an Engine cleanup capability for the
exact registry identity, validates ownership without following symlinks, checks
current authority before deletion and synchronizes removal. Published malformed
records quarantine; incomplete allocations are removable only when their bounded
contents prove ownership publication and source copying never completed. Cleanup
cannot resume a lost transaction or execute a check. Namespace locks coordinate
only liveness and quota accounting.

The tests actually SIGKILL a helper before ownership publication, before copying
an entry and after copying before validation, then reopen and recover through a
fresh Engine's cleanup-only grant. They also cover scope/store/identity mismatch,
revocation/expiry, live owners, runtime retention, corruption, symlink replacement
and quota exhaustion. Interrupted deletion that damages an ownership record
quarantines for operator review rather than inferring deletion authority. The
regression test retains the damaged copy and cannot turn that state into
successful evidence.

## Concrete T1 boundary

The reviewed development configuration uses rootless runc 1.5.2 on Linux x86_64
with delegated cgroup v2 and a live systemd user manager. The implementation
requires the validated user-service cgroup layout; other hosts fail closed when
those properties cannot be established. Host UID 0 execution is refused.

The runtime, control-plane property reader, entrypoint, formatter, interpreter
and libraries are individually content-pinned. The guest receives a closed file
image, private user/mount/PID/IPC/UTS/cgroup/network namespaces, empty capability
sets, no-new-privileges and an allowlisted seccomp profile. Source and the image
are read-only. The bounded writable scratch mount is separate, noexec, nosuid
and nodev. Source is also noexec, nosuid and nodev. No host home, Docker socket,
user bus, credentials or ambient executable search path is supplied.

The runner verifies effective namespace identities, capabilities, seccomp,
no-new-privileges, mappings, mount flags, rlimits, cgroup ceilings and independent
scope-deadline properties before releasing the trusted entrypoint to execute the
check. The observations are synchronized before release and retained with the
check artifacts. Engine admission is revalidated before release and throughout
supervision.

runc calls `Container.Run` before atomically publishing its PID file. The
supervisor therefore waits within the admitted wall deadline for an absent final
PID marker. It refuses invalid markers and still requires every effective
control check before command release. Delayed and permanently missing marker
scenarios have separate real probes. See the
[pinned upstream implementation](https://github.com/opencontainers/runc/blob/v1.5.2/utils_linux.go).

CPU rate/total time, memory, zero swap, process count, file descriptors, writable
disk, wall time, output and artifact budgets have distinct controls. Observed
resource counters are recorded as observations; missing samples do not establish
an exact lifetime resource measurement. Output exhaustion, ambiguous runtime exit
status, unknown termination or incomplete cleanup cannot become passing evidence.

An independent systemd scope deadline and whole-cgroup kill policy survive loss
of the controller. Runtime process ownership is checked before signalling;
pidfds avoid redirecting signals through PID reuse. Cleanup confirms termination
and runtime removal, or retains uncertainty. A container shares the host kernel;
this is T1 and does not provide microVM isolation. A compromised host kernel,
administrator or hostile process with the embedding's host UID is outside this
boundary.

T2/T3 execution, scoped network allowlists, credential delivery and custom output
handoff are unsupported by this runner and fail closed. There is no host-command
fallback. Reconcile owned runs before changing a backend generation: identity
binds the runtime, image, kernel/control profile and adapter/materializer source.
An old journal is not permission to reinterpret a different generation.

## Idempotency, storage and recovery

A durable launch claim binds scope, admission, attempt, check, plan, candidate,
backend, owned filesystem identity and runtime identity. Publication precedes
launch. The same execution identity cannot launch twice after response loss.
Preparation, cleanup-confirmation and terminal-record publication are separate
boundaries. A cleanup marker proves cleanup; it never fabricates a check result.

A cleanup-only capability can query or reconcile the exact owned journal. A
fresh operator store can stop an orphan after loss of the development Engine,
but cannot recreate its transaction or evidence authority. Versioned raw records
can be recovered without rerunning their checks. A response lost after Engine
retention is reconciled by exact evidence redelivery and its original receipt.

`LocalSourceArtifactStore::create/open` synchronize the validated store header,
root and parent directory. Object publication uses bounded private files,
content addresses, file/directory synchronization and verified readback. Reopen
and pending-object reconciliation preserve already published objects. A local
storage lock coordinates budget accounting only; it has no transaction authority.

Use persistent `create/open` for retained records. `temporary` is an explicit test
adapter whose final drop removes its files. Neither adapter is a distributed
object store, a retention daemon or protection against a host administrator.

## Actual harness and fixture scope

The concrete language check is gofmt from Go 1.26.2, with exact command
`/toolchain/gofmt -e /src/main.go`. It parses and renders source. It is not a
format-conformance lint, Go type checker, compiler, test suite or language-wide
Go harness. TypeScript/JavaScript, Python and Rust execution harnesses are not
claimed by this component.

The real integration fixture creates a controlled Git repository, captures an
immutable selected commit, reads repository JSON through `ConfiguredHarness`,
and binds configured check inputs to the candidate. It also constructs the
actual structural index and retains its incomplete result. Positive execution
uses a separately identified, manually established finite reference dependency
model for the exact controlled bytes/check inputs. Selecting checks does not
promote the structural index or establish complete program semantics.

## Reproduction

Portable Engine, repository and local filesystem checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --examples --locked
```

Actual OCI probes are explicit tests. Supply a reviewed private tooling directory
through `CELETIXO_OCI_HOME`; the runner never downloads or installs dependencies.
It contains `runc-1.5.2`, `celetixo-exec-init`, `toolchain/gofmt`,
`toolchain/lib/libc.so.6` and `toolchain/lib/ld-linux-x86-64.so.2`.
The fixture additionally pins the installed `/usr/bin/busctl` control-plane tool.

The runtime SHA-256 is
`599f6f94ff8c5057241eff0d54c3c74f95c34935b6457b33fe545defc61e9488`.
The formatter SHA-256 is
`0802a3784f7b2a924ccde5b34f6727ffe74e0193a1c0eea4cd773863fce4528a`.
Runtime signature verification, tool origin, compiler and library provenance must
be retained with a measured run. The entrypoint source is
`crates/execution/src/bin/celetixo-exec-init.rs`. Verify its resulting ELF
interpreter and required libraries; a static-linking compiler flag alone does
not prove a static binary. Different reviewed binaries produce a different
image identity.

With the required user manager and delegation available:

```sh
CELETIXO_OCI_HOME=/reviewed/private/tooling cargo test --locked -p celetixo-execution --test oci --test source_flow -- --ignored --test-threads=1
```

The tests include actual isolation/resource boundaries, source immutability,
cancellation, timeout, controller loss, owned cleanup, publication faults, PID
publication, mismatched-base rejection and fresh retry with preserved failed
history, plus patch-to-Engine-evidence integration. Ordinary filesystem and
finite fake-backend tests prove their stated contracts; they do not substitute
for these actual containment tests.

The [measurement protocol](../benchmarks/goal-03-protocol.md) covers actual-repository
T0 patch/copy probes and generated finite-reference T1 checks, evidence and
capacity/recovery probes. Representative results and targets remain a separate
exit gate:

```sh
CELETIXO_OCI_HOME=/reviewed/private/tooling cargo run --release --locked   -p celetixo-execution --example source_execution_bench -- --iterations 3
```

It records observed phases and exact input/check identities. Cache reuse is
disabled. A synchronous busy backend rejects rather than queues; no queue
latency or cold-cache result is inferred from a single run.

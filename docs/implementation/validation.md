# Goal 01 validation record

This record separates observations from planned checks. The [implementation contract](goal-01.md) defines the proof boundary and commands. A local pass does not imply that remote CI, deployment or distributed services have been validated.

## Initial inspection

The initial repository inspection found branch `main` at `c667e10`, a clean working tree and thirteen tracked Markdown files. There was no Cargo manifest, application implementation, executable test suite or CI configuration. Rust/Cargo 1.94.1, rustfmt and Clippy were available. These are baseline observations recorded before implementation, not descriptions of the completed worktree.

## Implementation milestones

| Milestone | Commit | Checks and result |
|---|---|---|
| Reproducible Rust workspace and domain foundation | `3a27fa6` | Format, strict Clippy, locked test and build checks passed; 15 unit + 11 snapshot tests |
| Transaction authority, OCC and evidence-gated reference commit | `24b32f8` | Format, strict Clippy, locked test and build checks passed; core integration suite: 23 tests |
| Deterministic histories, adversarial review and executable example | This milestone (hash recorded in audit record) | Full suite: 62 tests passed, including 192 generated histories; strict Clippy/build/format and example passed |

Populate this table only with actual commit identities and completed checks. A milestone may cite detailed results below. Do not infer passing status from a configured CI file or a previous command run against different code.

### Foundation evidence

Ran `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build --workspace --locked` successfully. Tests: 15 domain unit tests and 11 snapshot integration tests, no failures. The initially attempted locked test correctly refused to create an absent lockfile; `cargo generate-lockfile --offline` established the committed lockfile before the successful locked checks. The Engine and runnable concurrency example are subsequent milestones; they are not part of this foundation result.

### Engine evidence

The same four workspace commands passed after integrating the Engine. The core integration suite contains 23 passing tests: compatible commits and repeated revalidation, write/read/predicate/invariant conflicts, exact evidence checks, scoped authorization and revocation, lease expiry/renewal/reclamation/audit, idempotent outcomes, cancellation schedules, simultaneous commits through independent Engine handles, and five named commit failpoints. Together with foundation tests, this milestone contains 49 passing tests. Additional independent oracle/adversarial tests and the example passed in the development worktree and are committed in the next milestone; they are not counted as artifacts of this Engine commit.

### Reference histories and executable proof

The full locked suite passed with 15 domain unit tests, 11 snapshot tests, 23 Engine transaction tests, 9 adversarial regressions and 4 independent oracle tests: 62 passed, zero failed or ignored. The oracle uses bit masks and a reachability matrix, independently of the Engine's sets/work-list closure; it checks 192 fixed seeds and validates shrinking against an injected faulty oracle. Named regressions cover derived predicate/invariant inputs, capability fencing, verifier impersonation and stale-attempt evidence. A mismatch prints its seed and minimized history.

Review exposed missing closure roots beneath prefix reads and explicit invariant scopes. Both were fixed and regression-tested. Ownership and verifier authority were also reviewed: requesting analysis cannot replace model facts, reservation requires the proposer principal, and the gate rechecks live worker and verifier grants. Previous attempts and lease event history are exposed for audit.

The example performs actual in-process invariant evaluation and retains the evaluated state and PASS/FAIL transcript. It asserts compatible final state `x=10, y=20` at revision 2 and incompatible final state `x=10, y=2` at revision 1. No repository code or external verifier process is executed.

## Final commands

| Command | Observed result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed; zero warnings |
| `cargo test --workspace --locked` | Passed; 62 tests, zero failures |
| `cargo build --workspace --locked` | Passed |
| `cargo run --locked -p celetixo-engine --example concurrent_transactions` | Passed; all scenario assertions |

These results were observed in the development worktree using Rust/Cargo 1.94.1. A committed, detached-checkout verification is recorded in the completion audit below once executed.

## Requirement audit

| Goal 01 requirement | Authoritative implementation and executable evidence |
|---|---|
| Reproducible modular Rust workspace and CI | Root manifests/toolchain/lockfile; eight domain modules; `.github/workflows/ci.yml`; all five documented checks above |
| Typed scoped identity and immutable dirty-workspace contract | `ids.rs`, `snapshot.rs`; 11 snapshot tests include staged/unstaged/untracked/overlay/external inputs, canonicalization, modes and provenance |
| Legal transaction states and explicit rejection/revalidation | `transaction.rs`, Engine state mutations; terminal state unit test; abort, illegal order and multiple-attempt integration tests |
| Compatible concurrent work preserves both changes | Compatible scenario example; simultaneous independent Engine handles; repeated revalidation with archived attempts |
| Conflicting histories cannot silently commit | Write/write, invalidated reads, write-skew, predicate phantom, invariant and graph-derived dependency tests; 192 seeded oracle histories |
| Exact verification-gated commit | Immutable evidence and full impact digest; mismatch matrix, failed/incomplete/missing required checks, signer revocation, stale-attempt tests |
| Reservations and repeated operations | Lease owner/token checks, expiry/renewal/reclamation, monotonic clock, cancellation, recorded idempotent success and rejection tests |
| Deterministic faults and recovery | Five named commit failpoints, staged atomic publication and same-key lost-response recovery; ordered cancellation/revocation schedules |
| Atomicity assumptions and limits explicit | `goal-01.md`, Engine module documentation; single-store lifetime is required; no production distributed atomicity claim |
| Separate implementation commits and real results | Milestone table and commit history; no remote CI or comparative performance pass is claimed |

## Unvalidated targets

Remote GitHub CI, production distributed atomicity, process-restart durability, real repository capture, semantic extraction, supported-language harnesses, sandbox execution and deployed tenant isolation require their own implementation and validation evidence. This reference environment does not establish those properties. No benchmark or comparative evaluation result is implied by the correctness tests.

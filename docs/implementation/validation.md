# Goal 01 validation record

This record separates observations from planned checks. The [implementation contract](goal-01.md) defines the proof boundary and commands. A local pass does not imply that remote CI, deployment or distributed services have been validated.

## Initial inspection

The initial repository inspection found branch `main` at `c667e10`, a clean working tree and thirteen tracked Markdown files. There was no Cargo manifest, application implementation, executable test suite or CI configuration. Rust/Cargo 1.94.1, rustfmt and Clippy were available. These are baseline observations recorded before implementation, not descriptions of the completed worktree.

## Implementation milestones

| Milestone | Commit | Checks and result |
|---|---|---|
| Reproducible Rust workspace and domain foundation | `3a27fa6` | Format, strict Clippy, locked test and build checks passed; 15 unit + 11 snapshot tests |
| Transaction authority, OCC and evidence-gated reference commit | This milestone (hash recorded in next milestone) | Format, strict Clippy, locked test and build checks passed; core integration suite: 23 tests |
| Deterministic histories, failure recovery, example and final documentation | Pending | Not yet recorded |

Populate this table only with actual commit identities and completed checks. A milestone may cite detailed results below. Do not infer passing status from a configured CI file or a previous command run against different code.

### Foundation evidence

Ran `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and `cargo build --workspace --locked` successfully. Tests: 15 domain unit tests and 11 snapshot integration tests, no failures. The initially attempted locked test correctly refused to create an absent lockfile; `cargo generate-lockfile --offline` established the committed lockfile before the successful locked checks. The Engine and runnable concurrency example are subsequent milestones; they are not part of this foundation result.

### Engine evidence

The same four workspace commands passed after integrating the Engine. The core integration suite contains 23 passing tests: compatible commits and repeated revalidation, write/read/predicate/invariant conflicts, exact evidence checks, scoped authorization and revocation, lease expiry/renewal/reclamation/audit, idempotent outcomes, cancellation schedules, simultaneous commits through independent Engine handles, and five named commit failpoints. Together with foundation tests, this milestone contains 49 passing tests. Additional independent oracle/adversarial tests and the example passed in the development worktree and are committed in the next milestone; they are not counted as artifacts of this Engine commit.

## Final commands

| Command | Observed result |
|---|---|
| `cargo fmt --all -- --check` | Not yet recorded |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Not yet recorded |
| `cargo test --workspace --locked` | Not yet recorded |
| `cargo build --workspace --locked` | Not yet recorded |
| `cargo run --locked -p celetixo-engine --example concurrent_transactions` | Not yet recorded |

## Unvalidated targets

Remote GitHub CI, production distributed atomicity, process-restart durability, real repository capture, semantic extraction, supported-language harnesses, sandbox execution and deployed tenant isolation require their own implementation and validation evidence. This reference environment does not establish those properties. No benchmark or comparative evaluation result is implied by the correctness tests.

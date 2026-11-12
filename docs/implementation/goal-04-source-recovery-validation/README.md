# Source recovery validation

These are local development checks, not Engine-admitted verification evidence.
The implemented source recovery profile reconstructs normalized records through
a memory adapter. The actual PostgreSQL checks exercise the existing native
profile; they do not establish source SQL durability. Linux T1 checks use the
previously validated local execution profile.

`checks.json` records the commands, environment, source hashes, limitations and
results. `checks.log` is the command output. Private connection configuration,
credentials, database data and raw logs are retained outside the repository.
Public log presentation strips trailing whitespace and escapes NUL bytes; the
record retains both raw and presented log hashes. The correction log also trims
terminal blank lines for Git's diff check, retaining its earlier presentation hash.

## Findings retained during implementation

- An initial source-error codec change accidentally expanded the native outcome
  profile. Existing native rejection tests caught it. The final implementation
  preserves native formats and rejects source error families there; source
  outcomes have a separate typed wrapper.
- Clippy caught a test initialization style issue; the fixture was corrected.
- The first full workspace pass rejected stale exact public-input reviews for
  the changed Engine sources. `first-pass-checks.json` and
  `first-pass-checks.log` preserve that failure. The corrected fixtures bind exact
  current bytes. Default rejection and changed-byte rejection assertions remain.
- Fresh runtime recovery adds a source file containing numeric grant selector
  assignments. Its public review is scoped to the exact path and content digest.
  It contains no credentials. Both CI commands and the capture fixture use this
  eighth review; there is no broader admission exception.
- Engine-generated rejection diagnostics can exceed the worker event reason
  limit. Historical rejection validation now preserves the exact typed error
  under aggregate recovery budgets. Worker-authored event limits remain intact;
  a long-path rejection regression checks the distinction.
- Final review found that historical receipt reconstruction also needs the
  executor's immutable expiry check. The corrected validator rejects equality
  with receipt time; the corruption fixture checks equality rejection and the
  adjacent valid boundary. A subsequent Rust validation pass covers these final
  source bytes. The running native SQL regressions do not enter this source-only
  reconstruction function.
- The first correction pass stopped at rustfmt's required wrapping of the new
  test assignment. `correction-format-checks.json` and its log retain that result;
  `cargo fmt --all` applied it before restarting the correction checks.

## Source binding during the final run

After the runner captured its initial source manifest, two accidental literal
backslashes were removed from YAML folded commands in `.github/workflows/ci.yml`.
Final review then added the source-only receipt/executor expiry check and its
corruption assertion. The original runner deliberately reports the manifest
difference instead of declaring the entire tree unchanged.

Finalization compared every captured source file, retained that observation,
recorded the final manifest and checked the corrected workflow and its exact
eight reviewed-input arguments. `correction-checks.json` and its log
record focused, formatting, strict Clippy and workspace tests of the final Rust
bytes. Native SQL and Linux T1 regressions do not enter the changed historical
source reconstruction function; their production paths remained unchanged.
Workflow results bind to the final workflow bytes. A full-tree-unchanged claim
for the original run would be incorrect.

The final Rust pass passed 442 workspace tests with 45 opt-in tests ignored,
formatting, the focused expiry regression and strict workspace all-target Clippy.
The integration run passed 10 actual Linux T1 tests, all 32 PostgreSQL groups,
example builds/run, workflow checks, protocol integrity/four compatibility
mutation groups and 25 attribution fixtures. Six additional PostgreSQL child
result summaries are not added to the 32 parent groups. Of the 45 ordinary ignored
tests, 42 ran explicitly, two crash helpers run through their parents, and the
real rust-analyzer probe was not rerun.

`postgres-profile.json` records the observed PostgreSQL 17.9 fixture after the
suite through selected durability settings and authentication outcomes. The first
public audit rejected a literal authentication setting assignment under the
unchanged default policy; the full operator output is retained privately with
its hash in the public presentation. `postgres-stop.json` records successful
shutdown with data retained.
`public-input-review.json` records the final exact reviewed-input and default
public-file admission audit, including its separate self-admission check.

The original runner report is retained alongside the finalized report. The
complete Goal 04 contract and published migrations are unchanged. Hosted CI,
source SQL recovery, source prepared commit/revalidation and the remaining real
service and operational programme are outside this validation result.

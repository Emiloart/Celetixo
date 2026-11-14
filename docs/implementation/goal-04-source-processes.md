# Independent source commit processes

**Status:** validated bounded Linux/PostgreSQL process proof. This extends
milestone 04.2; complete 04.2 and Goal 04 acceptance are not claimed here.
[Checks](goal-04-source-process-validation/checks.json),
[output](goal-04-source-process-validation/checks.log) and the
[frozen source manifest](goal-04-source-process-validation/source-manifest.json)
record 550 ordinary workspace passes (117 environment-dependent cases ignored),
two selected real PostgreSQL parents, 10 actual Linux T1 regressions, strict
workspace Clippy, formatting, workflow semantics and diff checks.

Independent Engine processes can verify the same retained local artifacts before
competing at the PostgreSQL source commit boundary. Each process authenticates
its owner and independent analysis session and resolves the current scoped
artifact registration. No Session, reader object or process mutex is shared.
The source check records are synthetic fixtures; the objects and database are real.

`LocalSourceArtifactReader::open` exposes only verified reads from an existing
store. It checks the exact scope, retention, limits, private root/header, complete
bounded object census and persistence using the existing store implementation.
Each read retains digest/length, inode/mode/link and root-identity checks plus
file/directory synchronization. Reader construction and retrieval occur outside
the SQL transaction; current registration and authority are rechecked afterward.
The existing exclusive writer lock and aggregate write budgets remain unchanged.
A reader does not extend the lifetime of a temporary store or authorize writes,
execution, evidence admission or commit.

The process tests cover two callers using one original key, two original keys
competing on one source transaction, and process exit immediately before or after
SQL commit. They require one source revision/history, one terminal outcome and one
outbox/delivery fact. Exact retries return the original receipt or rejection.
Failed precommit publication leaves the complete checked scope unchanged.
Children use retained request/artifact files and private configuration selectors;
credential values do not enter request bytes or process arguments.

This is a local shared-filesystem reader, not an S3 implementation or a distributed
storage/retention service. The same-transaction races do not establish complete
independent-process histories involving distinct source transactions. Generated
source history/oracle expansion and the remaining original 04.2–04.7 acceptance
programme remain active. No hosted CI or production deployment result is inferred.

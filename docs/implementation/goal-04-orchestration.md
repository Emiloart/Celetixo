# Temporal orchestration

**Status:** 04.4 in progress. Go protocol intake and a scoped local Temporal host
are implemented. Authenticated Engine RPC and domain workflows remain required;
the synthetic coordination probe cannot establish Engine retry or commit behavior.
The full [Goal 04 contract](goal-04.md) is unchanged.

## Implemented service boundary

[The orchestration module](../../services/orchestration/go.mod) pins Go 1.26.11,
Temporal server 1.32.1, SDK 1.49.0, API 1.63.5 and protobuf 1.36.11. Generated Go
messages derive from the same canonical schemas and golden TaskIntent as Rust.
The bounded original-wire scanner rejects unknown fields, duplicate singular
fields, closed-enum violations, overflow and noncanonical encodings. It preserves
the full TaskIntent. Rust semantic intake, authenticated Engine authorization and
admission remain separate; decoding alone authorizes no action.

`celetixo-temporal {migrate|serve} --config /absolute/private/operator.json` embeds
the pinned vendor service. Explicit migration uses the vendor embedded schemas
in two separate PostgreSQL databases with separate workflow and visibility
ledgers. Serving never runs DDL. The runtime role is a nonowner without database
or schema CREATE, role administration, replication or RLS bypass. Host preflight
rejects elevated roles; database administrator credentials stay outside the host.
Existing partial schemas without a valid ledger fail closed.

The reviewed local database path uses SCRAM through a Unix socket. Network
PostgreSQL is unsupported by this configuration. Frontend and internode RPC use
TLS 1.3, required verified client certificates, hostname verification and separate
trust roots. Static explicit peers avoid an unauthenticated membership listener.
Private operator files must be canonical owned regular single-link mode-0600
files, nonempty and bounded. Password bytes are read in memory; commands and
workflow payloads do not contain credentials. Configuration is data only, with
unknown JSON/YAML keys and external password commands rejected.

[The identity plugin](../../services/orchestration/temporalauth/policy.go) maps
verified certificate fingerprints to explicit operator, client or worker bindings.
Every RPC checks certificate-chain validity again, including on an existing
connection. Namespaces hash the complete organization/workspace/repository tuple.
Clients and workers have distinct method allowlists and explicit task queues;
child, activity and continue-as-new commands cannot silently select a
foreign queue or namespace. Sticky queues reject because caller-supplied normal
queue names do not prove their association; the local worker disables sticky
caching. A future sticky profile needs a verified association. Caller-filled
identities or claims cannot grant a role. New unsupported methods and commands
reject. The separate privileged
system worker operates Temporal itself and has no Engine database credentials.
Policy replacement/revocation currently requires an operator-controlled host
restart. This is not a production workload-identity or revocation service.

## Reproduction and next acceptance boundary

The [local Temporal runbook](../operations/local-temporal-runbook.md) covers pinned
tooling, explicit provisioning, role separation, TLS checks, retained data and
the synthetic workflow recovery probe. The fixture runs on a private internal
Docker network with no published host ports, a read-only root and mounts, dropped
capabilities, no-new-privileges and bounded CPU, memory, processes, temporary
storage and logs. Container isolation is not a T2 microVM claim.

[Frozen validation receipts](goal-04-temporal-bootstrap/checks.json) record
module-content verification, generated binding reproduction, all Go package
tests, vet and formatting, workflow semantics, fresh vendor schema migration and
actual TLS/role probes. One exact synthetic workflow/run reaches a checkpoint,
survives worker SIGKILL and complete Temporal/PostgreSQL stop/restart, resumes from
its retained state, returns the complete canonical TaskIntent digest and replays
recorded history. Its replacement worker drains successfully on SIGTERM. The
initial probe used an invalid query-handler signature and failed; its raw output
is retained separately and is not counted as passing. Corrected sources and
artifact hashes accompany the final successful run.

The SDK probe uses the canonical full TaskIntent fixture as opaque workflow input.
Its activity strictly decodes/reencodes that message and returns its digest. It
does not admit an intent, run repository code, record verification, acquire a
reservation or commit. Recovery of this probe demonstrates coordination only.

04.4 still requires a canonical authenticated Rust Engine service boundary and
durable workflows/activities around real Engine operations. Preserve every
TaskIntent field, fresh scope/grant resolution and original request digests.
Activities must reconcile exact durable outcomes before retrying, use bounded
retry/deadline/heartbeat policies, and obtain authoritative cancellation/commit
outcomes. Validate side-effect timeout, worker death, replay, cancellation races
and deployment/version changes against actual Engine/PostgreSQL services, for
both native and captured-source paths. Workflow completion never supplies commit
authority. S3/governance, derived analytics/shared services and full operational
proof remain the original 04.5–04.7 work.

# Local lifecycle event delivery

**Scope:** trusted operator procedure for the bounded local PostgreSQL/NATS
profile in [the implementation record](../implementation/goal-04-events.md).
Docker and fixture administration are operator privileges, never worker inputs.
This procedure does not establish a production deployment or high availability.

## Fixtures and tests

Use Linux x86_64, the pinned Rust toolchain, Python 3, OpenSSL and a non-root
operator with Docker access. PostgreSQL prerequisites and private storage guards
are in [the fixture instructions](../../scripts/goal04/README.md). Provisioning
requires explicit network access to the selected official NATS release; subsequent
fixture operations use the pinned cached executable and image with no image pull.

```sh
python3 scripts/goal04/postgres_fixture.py start --root /absolute/private/pg17
python3 scripts/goal04/postgres_fixture.py prepare-events --root /absolute/private/pg17
python3 scripts/goal04/nats_fixture.py provision-tools --root /absolute/private/nats --tools /absolute/private/nats-tools
python3 scripts/goal04/nats_fixture.py start --root /absolute/private/nats --tools /absolute/private/nats-tools
python3 scripts/goal04/nats_fixture.py verify --root /absolute/private/nats --tools /absolute/private/nats-tools
export CELETIXO_PG_TEST_CONFIG=/absolute/private/pg17/connection-events.json
export CELETIXO_NATS_TEST_CONFIG=/absolute/private/nats/connection.json
cargo test --locked -p celetixo-postgres --test foundation native_authority::events:: -- --ignored --skip event_process_checkpoint --test-threads=1
cargo test --locked -p celetixo-postgres --test foundation source_lifecycle_frames_replay_every_phase -- --ignored --test-threads=1
```

The broker parent operates only its dedicated fixture stream/account. Each new
invocation resets its derived `PROJECTION` cursor for a new synthetic SQL scope;
it retains stream and SQL facts. Its crash/restart cases preserve that cursor
within the invocation. Do not point these tests at a shared or production broker.

`prepare-events` creates separate nonadministrative publisher/consumer logins and
private password files. It grants database connection only. Explicit scoped
bindings and least-privilege grants are installed by the test harness. Production
provisioning must perform equivalent reviewed administration; workload actors
cannot grant themselves access. Repository IDs in configuration are selectors,
not authorization.

## Worker configuration

Build `cargo build --locked -p celetixo-events`. The binary accepts exactly
`--config /absolute/private/service.json`. The file, database password and all
certificate/key inputs must be absolute canonical regular files owned by the
operator, mode `0600`, single link, nonempty and at most 64 KiB. Keep parent
directories private and protect them against replacement. Never pass passwords
in arguments, environment variables, event bodies or workflow payloads.

The JSON shape below contains placeholders only. Replace paths and identities
with independently provisioned private inputs. Use separate configurations and
logins for `Publisher` and `Consumer`.

```json
{
  "contract_version": 1,
  "organization_id": "organization",
  "workspace_id": "workspace",
  "repository_id": "repository",
  "service_id": "publisher-service",
  "service_kind": "Publisher",
  "database": {
    "directory": "/absolute/private/pg17/socket",
    "database": "celetixo",
    "user": "celetixo_event_publisher",
    "password_file": "/absolute/private/pg17/secrets/event-publisher.password"
  },
  "broker": {
    "servers": ["tls://broker-name:4222"],
    "ca_file": "/absolute/private/ca.pem",
    "certificate": "/absolute/private/publisher.pem",
    "private_key": "/absolute/private/publisher.key",
    "subject": "scope.lifecycle",
    "stream": "LIFECYCLE",
    "consumer": "PROJECTION",
    "inbox_prefix": "_INBOX.publisher"
  },
  "poll_interval_ms": 250,
  "reconcile_interval_ms": 1000
}
```

The database administrator maps login name **and OID** in `db_role_scopes` and
`event_service_roles` for the exact scope, service kind and service ID. Grant schema
usage, required narrow scope functions and SELECT on schema/repository/outbox/
frame/payload/delivery metadata. Publishers may UPDATE `outbox_delivery` only.
Consumers additionally read heads/transactions/inbox/projections/quarantine,
INSERT inbox/projections/quarantine and UPDATE projections. Neither role receives
Engine mutation, mapping-table, schema CREATE, owner membership or administrative
privileges. See the executable grant fixture in
[events.rs](../../crates/postgres/tests/foundation/native_authority/events.rs).

An independently authenticated broker operator provisions stream/consumer limits
and scope-specific account/subject ACLs. The worker cannot administer streams or
consumers. Require explicit ACK, max delivery 8, pending ACKs 1–64 and ACK wait
1–60 seconds. Set finite stream age/bytes/messages and three file replicas for
the tested profile. Publisher access covers its data subject and own reply inbox;
consumer access covers its durable INFO/NEXT/ACK subjects and own reply inbox.
Foreign accounts must not import/export these subjects. Client and route trust
use separate CAs; never mount signing keys or other clients' keys in brokers.

## Failure and replay

After PubAck, a publisher marks only its still-live fenced SQL claim. If it dies
before that marker, expiry makes the same stable event available again. SQL inbox
identity, rather than the finite NATS deduplication window, protects projection
effects. Consumer effects commit before ACK; a lost ACK replays the same receipt.
Consumers periodically reconcile missing SQL frames so retention or exhausted
redelivery cannot permanently hide authoritative facts.

Inspect worker numeric status and outbox pending/claimed/quarantined/unframed counts
and oldest pending age. `ready` describes the latest poll, not a zero-backlog or
zero-quarantine assertion; alert on quarantine, unframed rows, persistent lag and
retry exhaustion. Logs are local JSON, not an installed monitoring platform.
Unsupported versions and substituted bytes require operator diagnosis and a
compatible consumer deployment. Preserve immutable facts/quarantine. Do not edit
event bytes, migration ledgers, receipts or Engine truth to make delivery succeed.

SIGINT/SIGTERM stops after the bounded current batch and drains the client. The
worker exits after eight consecutive dependency failures; a supervised restart
reopens current scoped SQL authority. Unfinished claims expire; durable consumers
redeliver. Neither restart nor replay invokes the Engine commit operation.

```sh
python3 scripts/goal04/nats_fixture.py stop --root /absolute/private/nats --tools /absolute/private/nats-tools
python3 scripts/goal04/postgres_fixture.py stop --root /absolute/private/pg17
```

Stop retains data and credentials. Monitor real backing-volume space; fixture
budgets are not disk quotas. The official
[TLS guidance](https://docs.nats.io/learn/security/encryption),
[pinned server release](https://github.com/nats-io/nats-server/releases/tag/v2.15.3)
and [client API](https://docs.rs/async-nats/0.52.0/async_nats/jetstream/index.html)
support configuration review; actual Celetixo claims are limited to linked checks.

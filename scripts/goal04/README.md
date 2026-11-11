# Goal 04 operator fixtures

`postgres_fixture.py` starts the reviewed PostgreSQL 17 distribution for local
integration tests. It executes trusted database tooling and never repository
hooks, source programs or build scripts. It does not establish production
deployment, high availability or physical power-loss durability.

Prerequisites are Linux, Python 3, a non-root operator with access to a working
Docker daemon, and the exact cached image identified in the script. It never
pulls an image automatically. Acquire the selected immutable image explicitly if absent:

```sh
docker pull postgres@sha256:c7526c0f6c3f30260a563d7bcf8ad778effac59a44f8ffa86678c35418338609
``` Docker daemon access is privileged operator access;
do not expose this script or its socket to workers.

```sh
python3 scripts/goal04/postgres_fixture.py start
python3 scripts/goal04/postgres_fixture.py verify
python3 scripts/goal04/postgres_fixture.py status
python3 scripts/goal04/postgres_fixture.py restart
python3 scripts/goal04/postgres_fixture.py stop
```

Use `--root /absolute/private/path` after the operation to choose a separate
fixture. The Unix socket path must fit the Linux socket path limit. Start and
restart preserve the same data, credentials and container. Stop performs a bounded
graceful database stop; there is no delete or reset command. A partially initialized
data directory requires operator review and is never erased automatically.

The default fixture is under
`~/.local/share/celetixo-goal04/service-fixtures/postgres17`. Root, data, socket,
configuration and secret directories have mode `0700`; private files have mode
`0600`. The container runs as the operator's numeric UID/GID, with all capabilities
dropped, no new privileges, a read-only image, a bounded temporary directory and
no network namespace connectivity or published TCP ports. PostgreSQL listens only
on the private Unix socket and requires SCRAM authentication there. The fixture
does not offer network/TLS access; a network deployment needs separate transport,
identity and policy configuration and validation.

`connection.json` records `schema_version`, `host`, `port`, `database`, image
provenance, container name and the `admin`, `migration`, `runtime`, `runtime_other`
role entries. Each role contains `user` and `password_file`; passwords are separate
private files and are never printed. A Rust integration harness can read this path
through `CELETIXO_PG_TEST_CONFIG`. Never commit those files or place their contents
in command arguments, logs, workflow payloads or repository `.env` files.

The bootstrap administrator owns only the fixture. The migration role owns the
application database but is neither superuser nor a role administrator. Both
runtime roles initially receive only database connection permission. Schema
migrations and tests must explicitly grant table/schema privileges and install
scope policies; the fixture grants no generic runtime table access.

Durability settings retain `fsync=on`, `full_page_writes=on`,
`synchronous_commit=on`, checksums and durable host-backed PGDATA. Container limits
are one CPU, 384 MiB memory with no additional swap allowance, 96 processes,
32 MiB shared memory, 16 MiB temporary space and rotated logs of at most two
2 MiB segments. Database settings bound connection count, temporary query files
and memory. These are fixture controls, not measured capacity claims.

Start/status/verify/restart check PGDATA against a 1 GiB / 20,000-file guard and
require at least 512 MiB of free backing space. This guard and PostgreSQL's
`max_wal_size` are **not hard filesystem quotas**; WAL or concurrent writes can
overshoot between checks. Operators must monitor actual backing storage and stop
the fixture before exhaustion. WSL's large virtual ext4 free-space figure is not
proof that the Windows volume backing the virtual disk has equivalent space.
Do not run an unbounded load test using this fixture.

Verification independently checks live PostgreSQL settings, successful role
authentication, incorrect-password rejection, the host-visible SCRAM challenge,
nonprivileged application roles and container network identity. It does not
replace migration, multi-process race, crash/reconciliation, authorization,
backup/restore or failover tests required by Goal 04.

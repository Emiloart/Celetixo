# Local Temporal service and coordination probe

This is trusted operator tooling for non-root Linux x86_64 with local Unix-socket
Docker access. It operates a dedicated synthetic fixture. It does not deploy a
production service or establish the complete 04.4 Engine workflow contract. Read
the [implementation boundary](../implementation/goal-04-orchestration.md) and
[PostgreSQL fixture prerequisites](../../scripts/goal04/README.md) first.

Provision exact tools and modules with explicit network access, then compile:

```sh
python3 scripts/protocol/provision_go.py --cache-dir /absolute/private/tools
python3 scripts/protocol/provision.py --cache-dir /absolute/private/tools
export GOTOOLCHAIN=local GOMAXPROCS=2 GOFLAGS=-p=2 CGO_ENABLED=0
export GOMODCACHE=/absolute/private/go-module-cache
export GOCACHE=/absolute/private/go-build-cache
cd services/orchestration
/absolute/private/tools/go1.26.11/go/bin/go mod download
/absolute/private/tools/go1.26.11/go/bin/go mod verify
python3 ../../scripts/protocol/regenerate_go.py --offline --cache-dir /absolute/private/tools --check
/absolute/private/tools/go1.26.11/go/bin/go test -trimpath -buildvcs=false -mod=readonly ./...
/absolute/private/tools/go1.26.11/go/bin/go vet -trimpath -buildvcs=false -mod=readonly ./...
/absolute/private/tools/go1.26.11/go/bin/go build -trimpath -buildvcs=false -mod=readonly -o /absolute/private/celetixo-temporal ./cmd/celetixo-temporal
/absolute/private/tools/go1.26.11/go/bin/go build -trimpath -buildvcs=false -mod=readonly -o /absolute/private/celetixo-temporal-probe ./cmd/celetixo-temporal-probe
cd ../..
```

Keep build temporaries and cache on volumes with enough space; monitor the actual
backing drive. A large Temporal build can exceed a small RAM filesystem. Run
builds sequentially with two jobs, and retain exact source/toolchain hashes.
Normal Go builds use committed generated bindings and need no protoc.

Cache the reviewed PostgreSQL image explicitly, then prepare a fresh private root:

```sh
docker pull postgres@sha256:c7526c0f6c3f30260a563d7bcf8ad778effac59a44f8ffa86678c35418338609
python3 scripts/goal04/temporal_fixture.py prepare --root /absolute/private/temporal --binary /absolute/private/celetixo-temporal --probe /absolute/private/celetixo-temporal-probe --module-cache /absolute/private/go-module-cache
python3 scripts/goal04/temporal_fixture.py start --root /absolute/private/temporal
python3 scripts/goal04/temporal_fixture.py verify --root /absolute/private/temporal
python3 scripts/goal04/temporal_workflow_smoke.py --root /absolute/private/temporal
```

Preparation creates separate workflow and visibility databases, nonadministrative
migration/runtime roles, random private password files, independent frontend and
internode CAs, separate service/client/worker credentials and a scratch image with
the vendor license. It invokes migration in a separate bounded process. The
server mounts only its runtime inputs and own keys, never CA signing keys,
operator/client/worker keys or migration passwords. The operator's system-worker
credential is privileged inside Temporal and must remain private.

The manifest pins the absolute host/probe paths and their hashes. Keep those
artifacts unchanged while a fixture exists. Replacing a pinned artifact rejects;
the smoke helper can explicitly select a different operator-owned probe with
`--probe /absolute/private/probe`, recording its hash without replacing the
running service or its manifest. The server artifact remains pinned. Otherwise,
an upgrade needs explicit reviewed migration/version planning, not a silent
restart with substituted bytes. Partial preparation is preserved for diagnosis;
the helper never clears a migration ledger or deletes data to make startup pass.
Certificates last 30 days. Rotation is an explicit operator task; expired or
unmapped credentials are not replaced automatically.

Verification checks container restrictions and mounts, namespace/client/worker
authorization, unmapped certificate denial, wrong CA/hostname, missing client
certificate, TLS 1.2 rejection and rejection of a frontend client at the internode
listener. A valid internode identity is the positive control. The synthetic smoke
binds to the original workflow/run IDs, observes a checkpoint, SIGKILLs its worker,
stops/restarts Temporal and PostgreSQL with data retained, resumes the same run,
checks its canonical TaskIntent digest and replays recorded history. Its activity
has no Engine effects. Private detailed logs/configurations stay under the root.

```sh
python3 scripts/goal04/temporal_fixture.py status --root /absolute/private/temporal
python3 scripts/goal04/temporal_fixture.py stop --root /absolute/private/temporal
```

Stop affects only the labeled fixture and retains all data, credentials, network
and image. The selected one-node Temporal/four-shard profile is a functional test
configuration, not high availability, capacity acceptance, host-power-loss proof,
backup/restore validation or domain side-effect recovery. No repository program
receives these operator inputs. No Engine authority is implied by Temporal roles.

# Canonical protocol sources

`celetixo/**/*.proto` is the sole wire-schema source. ADR-0001 specifies required
TaskIntent semantics; `scripts/protocol/check.py` checks their preservation. The
current consumers are Rust TaskIntent intake and lifecycle event bindings with
strict decoding; the lifecycle consumer additionally checks scoped SQL authority
and uses authenticated JetStream delivery.
There is no RPC service or generated Go/Python consumer yet. Read the
[protocol reference](../docs/reference/protocol.md) before extending the boundary.

## Reproduce generated files

Use pinned Rust 1.94.1 on Linux x86_64, Python 3, and these commands from the repository
root. The first two steps acquire trusted tooling/dependencies and require network
access unless already cached. Neither executes captured repository source.

```sh
cargo fetch --manifest-path scripts/protocol/generator/Cargo.toml --locked
python3 scripts/protocol/provision.py
python3 scripts/protocol/regenerate.py --offline --check
python3 scripts/protocol/check.py
python3 -O scripts/protocol/test_compatibility.py
cargo test --locked -p celetixo-protocol
```

Remove `--check` only when intentionally updating derived artifacts after changing
canonical source. The generator runs locked/offline, invokes the hash-verified
protoc, generates prost Rust and strict wire metadata from its descriptor, formats
them with the pinned toolchain, creates the golden binary and records digests in
`generation.json`. Normal workspace builds use checked-in bindings and require no
protoc. The standalone generator has its own committed Cargo.lock. An explicit
`--cache-dir /absolute/path` selects a tooling cache for provisioning/regeneration.

The compiler comes from the official protobuf v33.3 Linux x86_64 release. The
archive and binary SHA-256 values are pinned in `scripts/protocol/provision.py`;
cache substitution rejects. Other provisioning platforms are not validated. A
pinned digest establishes exact selected bytes, not a general supply-chain audit.

## Compatibility and review

`compatibility/v1.json` is the first published descriptor baseline, not another
schema to edit by hand. Do not regenerate it to make a breaking change pass. CI
also compares the previous published descriptor when present. Additive fields
need documented safety semantics and strict-consumer negotiation; existing meaning
must not change. Required TaskIntent fields cannot be removed from this version.
Any future removal in another version must reserve its old names and numbers.

`descriptor.bin`, `fixtures/task_intent_envelope.bin` and
`crates/protocol/src/generated/*.rs` are generated outputs. The textproto fixture
is a public test request, contains no credentials and exercises every ADR field.
Production inputs remain untrusted after successful intake; authentication,
Engine authorization, admission, evidence and atomic commit are separate gates.

#!/usr/bin/env python3
"""Verify checked-in protocol artifacts and additive-only descriptor compatibility."""
import argparse
import hashlib
import json
import re
from pathlib import Path
import subprocess
import sys

from descriptor import compatible, summarize
from provision import BINARY_SHA256, VERSION

ROOT = Path(__file__).resolve().parents[2]


def check(root, against_ref=None):
    manifest = json.loads((root / "proto/generation.json").read_text())
    if (manifest["schema"] != "celetixo/protocol-generation/v1"
            or manifest["protoc_version"] != VERSION
            or manifest["protoc_binary_sha256"] != BINARY_SHA256
            or manifest["prost_build_version"] != "0.14.4"
            or manifest["generator_lock_sha256"] != hashlib.sha256(
                (root / "scripts/protocol/generator/Cargo.lock").read_bytes()).hexdigest()):
        raise ValueError("protocol generator provenance differs from the pinned toolchain")
    required = {str(p.relative_to(root)) for p in (root / "proto").rglob("*.proto")}
    required.update(str(p.relative_to(root)) for p in (root / "crates/protocol/src/generated").glob("*.rs"))
    required.update({"proto/descriptor.bin", "proto/fixtures/task_intent_envelope.bin"})
    if set(manifest["artifacts"]) != required:
        raise ValueError("protocol manifest omits or adds an unrecognized generated artifact")
    for relative, expected in manifest["artifacts"].items():
        path = root / relative
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError("generated protocol artifact digest mismatch: " + relative)
    current = summarize((root / "proto/descriptor.bin").read_bytes())
    # The first schema preserves the accepted ADR exactly, including nested
    # message names, field types, repetition and field numbers.
    adr = (root / "docs/adr/0001-celetixo-engineering-state-coordination.md").read_text()
    types = {"string": 9, "uint32": 13, "uint64": 4, "bool": 8}
    for name, body in re.findall(r"message\s+(\w+)\s*\{([^}]+)\}", adr):
        published = current["messages"].get("celetixo.orchestration.v1." + name)
        if published is None:
            raise ValueError("ADR message omitted: " + name)
        expected = []
        for repeated, field_type, field_name, number in re.findall(r"\b(?:(repeated)\s+)?(\w+)\s+(\w+)\s*=\s*(\d+)\s*;", body):
            expected.append((field_name, int(number), 3 if repeated else 1, types.get(field_type, 11), "" if field_type in types else ".celetixo.orchestration.v1." + field_type))
        actual = {(f["name"],f["number"],f["label"],f["type"],f["type_name"]) for f in published}
        if not set(expected).issubset(actual):
            raise ValueError("canonical schema differs from accepted ADR fields: " + name)
    baseline = json.loads((root / "proto/compatibility/v1.json").read_text())
    compatible(baseline, current)
    if against_ref:
        # The caller chooses a trusted CI base SHA. Argument arrays avoid shell
        # interpolation; git objects are read only, never checked out/executed.
        old = subprocess.run(
            ["git", "show", against_ref + ":proto/descriptor.bin"], cwd=root,
            capture_output=True, check=False,
        )
        if old.returncode:
            raise ValueError("requested compatibility base has no readable canonical descriptor")
        compatible(summarize(old.stdout), current)
    return len(current["messages"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--against-ref")
    args = parser.parse_args()
    count = check(ROOT, args.against_ref)
    print(f"protocol integrity and compatibility passed ({count} canonical messages)")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)

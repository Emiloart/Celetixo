#!/usr/bin/env python3
"""Generate every derived protocol artifact from the sole canonical proto tree."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

from descriptor import summarize
from provision import BINARY_SHA256, VERSION, provision

ROOT = Path(__file__).resolve().parents[2]


def strict_fields(descriptor):
    messages = descriptor["messages"]
    names = {name: re.sub(r"(?<!^)(?=[A-Z])", "_", name.rsplit(".", 1)[-1]).upper() for name in messages}
    lines = ["// @generated from proto/descriptor.bin; never edit field metadata by hand."]
    for name, fields in sorted(messages.items()):
        lines.append("const " + names[name] + ": &[Field] = &[")
        for field in fields:
            kind = {9: "Text", 12: "Bytes", 13: "Uint32", 4: "Uint64", 8: "Bool"}.get(field["type"])
            if field["type"] == 11:
                kind = "Nested(" + names[field["type_name"].lstrip(".")] + ")"
            if field["type"] == 14:
                entries = descriptor["enums"][field["type_name"].lstrip(".")]
                kind = "Enum(&[" + ", ".join(str(entry["number"]) for entry in entries) + "])"
            if kind is None or field["label"] not in (1, 3):
                raise ValueError("strict intake does not support the requested schema field")
            lines.append(f"    ({field['number']}, {kind}, {str(field['label'] == 3).lower()}),")
        lines.append("];")
    lines.append("const ENVELOPE: &[Field] = TASK_INTENT_ENVELOPE;")
    return "\n".join(lines) + "\n"


def generate(output, protoc):
    subprocess.run(["cargo", "run", "--manifest-path", str(ROOT / "scripts/protocol/generator/Cargo.toml"), "--locked", "--offline", "--", str(ROOT), str(protoc), str(output)], cwd=ROOT, check=True)
    descriptor = summarize((output / "descriptor.bin").read_bytes())
    (output / "rust/strict_fields.rs").write_text(strict_fields(descriptor))
    for source in sorted((output / "rust").glob("*.rs")):
        subprocess.run(["rustfmt", "--edition", "2024", str(source)], cwd=ROOT, check=True)
    result = subprocess.run([str(protoc), "-I", str(ROOT / "proto"), "--encode=celetixo.platform.v1.TaskIntentEnvelope", "celetixo/platform/v1/task_intent_envelope.proto"], input=(ROOT / "proto/fixtures/task_intent_envelope.textproto").read_bytes(), capture_output=True, check=True)
    outputs = {"proto/descriptor.bin": (output / "descriptor.bin").read_bytes(), "proto/fixtures/task_intent_envelope.bin": result.stdout}
    outputs.update({"crates/protocol/src/generated/" + p.name: p.read_bytes() for p in sorted((output / "rust").glob("*.rs"))})
    artifacts = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((ROOT / "proto").rglob("*.proto"))}
    artifacts.update({name: hashlib.sha256(data).hexdigest() for name, data in outputs.items()})
    manifest = {"schema": "celetixo/protocol-generation/v1", "protoc_version": VERSION, "protoc_binary_sha256": BINARY_SHA256, "prost_build_version": "0.14.4", "generator_lock_sha256": hashlib.sha256((ROOT / "scripts/protocol/generator/Cargo.lock").read_bytes()).hexdigest(), "artifacts": dict(sorted(artifacts.items()))}
    outputs["proto/generation.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=Path.home() / ".cache/celetixo/protocol")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    protoc = provision(args.cache_dir, args.offline)
    with tempfile.TemporaryDirectory() as tmp:
        outputs = generate(Path(tmp), protoc)
        for relative, data in outputs.items():
            target = ROOT / relative
            if args.check:
                if not target.exists() or target.read_bytes() != data:
                    raise ValueError("generated artifact differs: " + relative)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
    print("canonical protocol regeneration " + ("verified" if args.check else "complete"))


if __name__ == "__main__":
    main()

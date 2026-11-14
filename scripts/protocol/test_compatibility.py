#!/usr/bin/env python3
"""Independent mutations verify that compatibility checks actually reject changes."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from check import check
from descriptor import compatible, fields, summarize

ROOT = Path(__file__).resolve().parents[2]


class Compatibility(unittest.TestCase):
    def test_checked_in_integrity_covers_schema_bindings_and_generator_provenance(self):
        check(ROOT)
        for kind in ["schema", "binding", "omission", "generator"]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for directory in ["proto", "crates/protocol/src/generated", "docs/adr"]:
                    shutil.copytree(ROOT / directory, root / directory)
                lock = root / "scripts/protocol/generator/Cargo.lock"
                lock.parent.mkdir(parents=True)
                shutil.copyfile(ROOT / "scripts/protocol/generator/Cargo.lock", lock)
                manifest_path = root / "proto/generation.json"
                manifest = json.loads(manifest_path.read_text())
                if kind == "omission":
                    manifest["artifacts"].pop("proto/celetixo/orchestration/v1/task_intent.proto")
                    manifest_path.write_text(json.dumps(manifest))
                else:
                    selected = {"schema": root / "proto/celetixo/orchestration/v1/task_intent.proto",
                                "binding": root / "crates/protocol/src/generated/strict_fields.rs",
                                "generator": lock}[kind]
                    selected.write_bytes(selected.read_bytes() + b"\nmodified\n")
                with self.assertRaises(ValueError):
                    check(root)

    def test_published_fields_cannot_be_removed_retyped_renamed_or_reused(self):
        baseline = json.loads((ROOT / "proto/compatibility/v1.json").read_text())
        name = "celetixo.orchestration.v1.TaskIntent"
        for kind in ["remove", "rename", "number", "type", "package", "message"]:
            with self.subTest(kind=kind):
                current = copy.deepcopy(baseline)
                if kind == "remove":
                    current["messages"][name].pop()
                elif kind == "message":
                    del current["messages"][name]
                elif kind == "package":
                    current["files"][0]["package"] = "unrelated.v1"
                else:
                    current["messages"][name][0][{"rename": "name", "number": "number", "type": "type"}[kind]] = {"rename": "replacement", "number": 99, "type": 4}[kind]
                with self.assertRaises(ValueError):
                    compatible(baseline, current)

    def test_additive_field_preserves_existing_meaning(self):
        baseline = json.loads((ROOT / "proto/compatibility/v1.json").read_text())
        current = copy.deepcopy(baseline)
        field = copy.deepcopy(current["messages"]["celetixo.orchestration.v1.TaskIntent"][0])
        field.update(name="future_field", json_name="futureField", number=17)
        current["messages"]["celetixo.orchestration.v1.TaskIntent"].append(field)
        compatible(baseline, current)

    def test_descriptor_reader_rejects_truncation_and_resource_amplification(self):
        for data in [b"\x0a\xff\xff\xff\xff\x7f", b"\x00", b"\x0a\x03\x01", b"\x09\x01", b"\x08" + b"\xff" * 11]:
            with self.subTest(data=data), self.assertRaises(ValueError):
                fields(data)
        current = summarize((ROOT / "proto/descriptor.bin").read_bytes())
        self.assertEqual(len(current["messages"]["celetixo.orchestration.v1.TaskIntent"]), 16)

    def test_enum_values_cannot_be_removed_renamed_renumbered_or_aliased(self):
        baseline = summarize((ROOT / "proto/descriptor.bin").read_bytes())
        name = "celetixo.platform.v1.TransactionLifecycleState"
        self.assertEqual(len(baseline["enums"][name]), 9)
        for kind in ["remove", "enum", "rename", "number", "alias"]:
            with self.subTest(kind=kind):
                current = copy.deepcopy(baseline)
                entries = current["enums"][name]
                if kind == "remove":
                    entries.pop()
                elif kind == "enum":
                    del current["enums"][name]
                elif kind == "alias":
                    entries.append({"name": "ALIAS", "number": 1})
                elif kind == "rename":
                    entries[1]["name"] = "REPLACEMENT"
                else:
                    entries[1]["number"] = 99
                with self.assertRaises(ValueError):
                    compatible(baseline, current)
        current = copy.deepcopy(baseline)
        current["enums"][name].append({"name": "FUTURE", "number": 99})
        compatible(baseline, current)


if __name__ == "__main__":
    unittest.main()

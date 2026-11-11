#!/usr/bin/env python3
"""Exercise the reporting boundary against retained raw benchmark records."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "docs/benchmarks/goal-03"
SCRIPT = DATA / "summarize.py"
INPUTS = ("run.json", "celetixo-t0.json", "xxhash-t0.json", "generated-t1.json")


class SummaryValidation(unittest.TestCase):
    def invoke(self, directory, optimized):
        return subprocess.run(
            [sys.executable, *(["-O"] if optimized else []), str(SCRIPT), str(directory)],
            capture_output=True,
            text=True,
            timeout=15,
        )

    def test_retained_results_reproduce_in_both_python_modes(self):
        for optimized in (False, True):
            with self.subTest(optimized=optimized), tempfile.TemporaryDirectory() as tmp:
                directory = Path(tmp)
                for name in INPUTS:
                    (directory / name).write_bytes((DATA / name).read_bytes())
                result = self.invoke(directory, optimized)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    (directory / "summary.json").read_bytes(),
                    (DATA / "summary.json").read_bytes(),
                )

    def test_invalid_measurements_never_publish_a_summary(self):
        changes = (
            ("failed command", "run.json", lambda d: d["commands"][0].update(exit=1)),
            ("no commands", "run.json", lambda d: d.update(commands=[])),
            ("dirty before", "run.json", lambda d: d.update(clean_worktree_before=False)),
            ("dirty after", "run.json", lambda d: d.update(clean_worktree_after=False)),
            ("too few T0 samples", "celetixo-t0.json", lambda d: d["samples"].pop()),
            ("mixed T0 base", "xxhash-t0.json", lambda d: d["samples"][0].update(base_snapshot="wrong")),
            ("too few T1 samples", "generated-t1.json", lambda d: d["samples"].pop()),
            ("mixed T1 base", "generated-t1.json", lambda d: d["samples"][0].update(base_snapshot="wrong")),
            ("cache reused", "generated-t1.json", lambda d: d["samples"][0].update(cache_reuse_enabled=True)),
            ("termination unknown", "generated-t1.json", lambda d: d["capacity_and_recovery"][0].update(termination_confirmed=False)),
            ("source retained", "generated-t1.json", lambda d: d["capacity_and_recovery"][0].update(owned_source_removed=False)),
            ("recovered check", "generated-t1.json", lambda d: d["capacity_and_recovery"][0].update(check_result_recovered=True)),
            ("queued attempt", "generated-t1.json", lambda d: d["capacity_and_recovery"][0].update(queued_attempts=1)),
        )
        for optimized in (False, True):
            for name, target, mutate in changes:
                with self.subTest(case=name, optimized=optimized), tempfile.TemporaryDirectory() as tmp:
                    directory = Path(tmp)
                    for source in INPUTS:
                        (directory / source).write_bytes((DATA / source).read_bytes())
                    value = json.loads((directory / target).read_text())
                    mutate(value)
                    (directory / target).write_text(json.dumps(value))
                    sentinel = b"existing report must survive invalid input\n"
                    (directory / "summary.json").write_bytes(sentinel)
                    result = self.invoke(directory, optimized)
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertEqual((directory / "summary.json").read_bytes(), sentinel)


if __name__ == "__main__":
    unittest.main()

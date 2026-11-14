#!/usr/bin/env python3
"""Recover one synthetic SDK workflow across worker death and service restart.

Trusted local fixture only. The activity hashes the canonical TaskIntent fixture;
it never invokes Engine operations or establishes domain workflow acceptance.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import time

import postgres_fixture as pg
import temporal_fixture

ROOT = Path(__file__).resolve().parents[2]


def execute(root, probe=None):
    fixture = temporal_fixture.Fixture(root)
    manifest = fixture.manifest()
    if probe is not None:
        # An explicit operator-selected diagnostic/SDK probe may change without
        # replacing a running server or its pinned artifact. Record exact bytes.
        manifest = {**manifest, "probe": str(probe), "probe_sha256": temporal_fixture.digest(probe)}
    fixture.probe(manifest, "health")
    directory = root / "logs" / ("workflow-" + secrets.token_hex(8))
    pg.private_directory(directory)
    canonical = (ROOT / "proto/fixtures/task_intent_envelope.bin").read_bytes()
    intent = directory / "intent.bin"
    descriptor = os.open(intent, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(canonical)
        output.flush()
        os.fsync(output.fileno())
    configuration = json.loads(pg.private_read(root / "clients/client.json"))
    configuration.update(probe_workflow_id="coordination-" + secrets.token_hex(16), probe_intent_file=str(intent))
    client_path = directory / "client.json"
    pg.private_write(client_path, json.dumps(configuration))
    worker_path = root / "clients/worker.json"
    checks = []
    binary = manifest["probe"]

    def invoke(label, mode, config=client_path):
        start = time.monotonic()
        result = subprocess.run([binary, mode, "--config", str(config)], capture_output=True, timeout=50)
        pg.private_write(directory / (label + ".raw.log"), (result.stdout + result.stderr).decode())
        receipt = {"check": label, "exit_code": result.returncode, "duration_seconds": round(time.monotonic() - start, 3),
                   "raw_log_sha256": hashlib.sha256(result.stdout + result.stderr).hexdigest()}
        checks.append(receipt)
        if result.returncode:
            raise RuntimeError("workflow smoke failed at " + label + "; private diagnostics retained")
        return json.loads(result.stdout)

    def worker(label):
        path = directory / (label + ".raw.log")
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        output = os.fdopen(descriptor, "wb")
        process = subprocess.Popen([binary, "run-worker", "--config", str(worker_path)], stdout=output, stderr=subprocess.STDOUT)
        output.close()
        deadline = time.monotonic() + 25
        try:
            while True:
                rows = path.read_text().splitlines()
                if any('"mode":"sdk-worker-ready"' in line for line in rows):
                    return process, path
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("SDK worker readiness failed; private diagnostics retained")
                time.sleep(0.1)
        except BaseException:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            raise

    process, first_log = worker("worker-before-crash")
    try:
        started = invoke("start", "start-probe")
        configuration["probe_run_id"] = started["run_id"]
        pg.private_write(client_path, json.dumps(configuration))
        invoke("checkpoint-before", "checkpoint-probe")
        process.kill()
        if process.wait(timeout=10) != -signal.SIGKILL:
            raise RuntimeError("worker did not exit at injected process death")
        checks.append({"check": "worker-sigkill", "exit_code": -signal.SIGKILL,
                       "raw_log_sha256": hashlib.sha256(first_log.read_bytes()).hexdigest()})
        start = time.monotonic()
        fixture.stop()
        fixture.start()
        checks.append({"check": "temporal-and-postgres-stop-restart", "duration_seconds": round(time.monotonic() - start, 3),
                       "data_retained": True})
        process, second_log = worker("worker-after-restart")
        invoke("checkpoint-after", "checkpoint-probe")
        invoke("complete-original-run", "wait-probe")
        invoke("replay-recorded-history", "replay-probe")
        process.send_signal(signal.SIGTERM)
        if process.wait(timeout=15) != 0 or '"mode":"sdk-worker-stopped"' not in second_log.read_text():
            raise RuntimeError("SDK worker graceful shutdown failed")
        checks.append({"check": "worker-sigterm", "exit_code": 0,
                       "raw_log_sha256": hashlib.sha256(second_log.read_bytes()).hexdigest()})
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
    result = {"schema": "celetixo/temporal-coordination-smoke/v1", "workflow_id": started["workflow_id"],
              "original_run_id": started["run_id"], "canonical_fixture_sha256": hashlib.sha256(canonical).hexdigest(),
              "binary_sha256": manifest["probe_sha256"], "host_sha256": manifest["binary_sha256"], "checks": checks,
              "private_log_directory": str(directory),
              "boundary": "Synthetic coordination only; no Engine side effect, domain retries/cancellation or complete 04.4 claim"}
    pg.private_write(directory / "checks.json", json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--probe", type=Path)
    args = parser.parse_args()
    pg.private_read(args.root / "fixture.lock")
    with (args.root / "fixture.lock").open("r+") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        print(json.dumps(execute(args.root, args.probe), indent=2))


if __name__ == "__main__":
    main()

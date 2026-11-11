#!/usr/bin/env python3
"""Check GitHub Actions semantics with a pinned actionlint; never run workflow jobs."""
import argparse
import hashlib
import io
from pathlib import Path
import platform
import subprocess
import tarfile
import urllib.request

VERSION = "1.7.12"
ARCHIVE_SHA256 = "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"
URL = f"https://github.com/rhysd/actionlint/releases/download/v{VERSION}/actionlint_{VERSION}_linux_amd64.tar.gz"
ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=Path.home() / ".cache/celetixo/workflow-lint")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        raise SystemExit("the pinned workflow checker requires Linux x86_64")
    cache = args.cache_dir.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f"actionlint_{VERSION}.tar.gz"
    if not archive.exists():
        if args.offline:
            raise SystemExit("pinned workflow checker archive is not cached")
        request = urllib.request.Request(URL, headers={"User-Agent": "Celetixo-workflow-validation"})
        with urllib.request.urlopen(request, timeout=30) as response:
            content = response.read(8 * 1024 * 1024 + 1)
        if len(content) > 8 * 1024 * 1024 or hashlib.sha256(content).hexdigest() != ARCHIVE_SHA256:
            raise SystemExit("workflow checker archive failed its pinned digest/length budget")
        archive.write_bytes(content)
    content = archive.read_bytes()
    if hashlib.sha256(content).hexdigest() != ARCHIVE_SHA256:
        raise SystemExit("cached workflow checker archive digest differs")
    with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as bundle:
        member = bundle.getmember("actionlint")
        if not member.isfile() or member.size > 16 * 1024 * 1024:
            raise SystemExit("workflow checker executable is not a bounded regular file")
        binary = bundle.extractfile(member).read()
    executable = cache / "actionlint"
    executable.write_bytes(binary)
    executable.chmod(0o755)
    print(f"actionlint {VERSION}, archive SHA-256 {ARCHIVE_SHA256}", flush=True)
    command = [str(executable), "-shellcheck=", "-pyflakes="]
    # Exercise the semantic error that a YAML-only parse failed to catch. The
    # linter parses data only; it does not execute this synthetic job.
    invalid = "on: push\njobs:\n  probe:\n    runs-on: ubuntu-latest\n    env:\n      INVALID: ${{ runner.temp }}\n    steps:\n      - run: echo probe\n"
    negative = subprocess.run(command + ["-"], input=invalid, capture_output=True, text=True, cwd=ROOT)
    if negative.returncode != 1 or 'context "runner" is not allowed here' not in negative.stdout + negative.stderr:
        raise SystemExit("workflow checker did not reject the invalid job-level runner context")
    subprocess.run(command, cwd=ROOT, check=True)
    print("Workflow semantics and invalid-context regression passed")


if __name__ == "__main__":
    main()

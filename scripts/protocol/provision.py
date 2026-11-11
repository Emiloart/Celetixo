#!/usr/bin/env python3
"""Acquire the exact official Linux x86_64 protoc release for regeneration."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import urllib.request
import zipfile

VERSION = "33.3"
URL = "https://github.com/protocolbuffers/protobuf/releases/download/v33.3/protoc-33.3-linux-x86_64.zip"
ARCHIVE_SHA256 = "2dbe6e2165f7721a58c2dc9e40fb47c5e3c2d63fb420c1f497db4ad3eb310ff8"
BINARY_SHA256 = "2dc9c631782f1c9b80b6dddd27f6402d5a23a073f0dd1ac6ca82acbf8a0f0719"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verified_binary(path):
    if not path.is_file() or digest(path) != BINARY_SHA256:
        raise RuntimeError("protoc executable does not match the pinned digest")
    result = subprocess.run([str(path), "--version"], check=True, capture_output=True, text=True)
    if result.stdout.strip() != "libprotoc " + VERSION:
        raise RuntimeError("unexpected protoc version")
    return path


def provision(cache, offline=False):
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        raise RuntimeError("reviewed codegen provisioner supports Linux x86_64 only")
    cache = cache.resolve()
    binary = cache / ("protoc-" + VERSION) / "bin" / "protoc"
    if binary.exists():
        return verified_binary(binary)
    archive = cache / ("protoc-" + VERSION + "-linux-x86_64.zip")
    cache.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        if offline:
            raise RuntimeError("pinned protoc archive is absent in offline mode")
        request = urllib.request.Request(URL, headers={"User-Agent": "Celetixo-protocol-tooling"})
        with urllib.request.urlopen(request, timeout=30) as response:
            content = response.read(8 * 1024 * 1024 + 1)
        if len(content) > 8 * 1024 * 1024 or hashlib.sha256(content).hexdigest() != ARCHIVE_SHA256:
            raise RuntimeError("downloaded protoc archive failed its pinned digest/length budget")
        with tempfile.NamedTemporaryFile(dir=cache, delete=False) as pending:
            pending.write(content)
            pending_name = pending.name
        os.replace(pending_name, archive)
    if digest(archive) != ARCHIVE_SHA256:
        raise RuntimeError("cached protoc archive does not match the pinned digest")
    with zipfile.ZipFile(archive) as bundle:
        # Extract exactly one reviewed path, never archive-controlled directories.
        content = bundle.read("bin/protoc")
    if hashlib.sha256(content).hexdigest() != BINARY_SHA256:
        raise RuntimeError("archive executable does not match the pinned digest")
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_bytes(content)
    binary.chmod(0o755)
    return verified_binary(binary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=Path.home() / ".cache/celetixo/protocol")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    print(provision(args.cache_dir, args.offline))


if __name__ == "__main__":
    main()

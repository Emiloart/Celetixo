#!/usr/bin/env python3
"""Provision the exact Linux/amd64 Go compiler used by orchestration tooling."""
import argparse
import hashlib
from pathlib import Path
import platform
import tarfile
import urllib.request

VERSION = "go1.26.11"
SIZE = 66999944
SHA256 = "9849f4e04ee35303714e34fbc3ac8ab44ea3e448a95aa182f91e3b9b79c710ec"
DEFAULT_CACHE = Path.home() / ".cache/celetixo/protocol"


def provision(cache, offline=False):
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        raise ValueError("the pinned Go tooling profile requires Linux x86_64")
    cache = cache.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / (VERSION + ".linux-amd64.tar.gz")
    if not archive.exists():
        if offline:
            raise ValueError("pinned Go archive is not cached")
        url = "https://go.dev/dl/" + archive.name
        with urllib.request.urlopen(url, timeout=30) as response:
            content = response.read(SIZE + 1)
        if len(content) != SIZE or hashlib.sha256(content).hexdigest() != SHA256:
            raise ValueError("Go archive failed pinned size/digest verification")
        archive.write_bytes(content)
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size != SIZE:
        raise ValueError("Go archive is not the expected bounded regular file")
    content = archive.read_bytes()
    if len(content) != SIZE or hashlib.sha256(content).hexdigest() != SHA256:
        raise ValueError("cached Go archive differs from the pinned release")
    target = cache / VERSION
    # Compare every extracted file on reuse, including the compiler and standard
    # library. Version output alone cannot detect a substituted tool/cache.
    with tarfile.open(archive, "r:gz") as bundle:
        members = bundle.getmembers()
        if len(members) > 30000 or sum(m.size for m in members) > 1024**3:
            raise ValueError("Go distribution exceeds the extraction budget")
        for member in members:
            parts = Path(member.name).parts
            if (not parts or parts[0] != "go" or ".." in parts
                    or Path(member.name).is_absolute()
                    or not (member.isfile() or member.isdir())):
                raise ValueError("unexpected Go archive path/member")
        if target.exists():
            if target.is_symlink() or not target.is_dir():
                raise ValueError("Go tool cache root is not a regular directory")
            expected = {m.name for m in members if m.isfile()}
            actual = set()
            for selected in target.rglob("*"):
                if selected.is_symlink() or not (selected.is_file() or selected.is_dir()):
                    raise ValueError("unexpected Go tool cache member")
                if selected.is_file():
                    actual.add(str(selected.relative_to(target)))
            if actual != expected:
                raise ValueError("Go tool cache omits or adds distribution files")
            for member in members:
                selected = target / member.name
                if selected.is_symlink():
                    raise ValueError("Go tool cache contains a symlink")
                if member.isfile():
                    if (not selected.is_file() or selected.stat().st_size != member.size
                            or selected.read_bytes() != bundle.extractfile(member).read()):
                        raise ValueError("Go tool cache differs from its verified archive")
        else:
            target.mkdir(mode=0o700)
            bundle.extractall(target, filter="data")
    return target / "go/bin/go"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    print(provision(args.cache_dir, args.offline))


if __name__ == "__main__":
    main()

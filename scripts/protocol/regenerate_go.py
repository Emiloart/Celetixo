#!/usr/bin/env python3
"""Generate Go bindings from the same canonical sources as Rust; no wire fork."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from provision import BINARY_SHA256, provision
from provision_go import DEFAULT_CACHE, VERSION, provision as provision_go

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "services/orchestration"
PREFIX = "github.com/Emiloart/Celetixo/services/orchestration"
FILES = {
    "celetixo/orchestration/v1/task_intent.proto": "orchestrationv1",
    "celetixo/platform/v1/task_intent_envelope.proto": "platformv1",
    "celetixo/platform/v1/lifecycle_event.proto": "platformv1",
}


def generate(cache, offline):
    go = provision_go(cache, offline)
    protoc = provision(cache, offline)
    environment = {**os.environ, "GOTOOLCHAIN": "local", "GOMAXPROCS": "2",
                   "GOROOT": str(go.parent.parent), "GOENV": "off", "GOWORK": "off",
                   "GOFLAGS": "", "GOEXPERIMENT": "", "CGO_ENABLED": "0",
                   "GOOS": "linux", "GOARCH": "amd64", "GOAMD64": "v1",
                   "GOPRIVATE": "", "GONOSUMDB": "", "GOVCS": "*:off",
                   "GOPROXY": "https://proxy.golang.org", "GOSUMDB": "sum.golang.org"}
    if offline:
        environment.update(GOPROXY="off", GOSUMDB="off")
    selected_version = subprocess.check_output(
        [str(go), "list", "-m", "-mod=readonly", "-f", "{{.Version}}", "google.golang.org/protobuf"],
        cwd=MODULE, env=environment, text=True).strip()
    if selected_version != "v1.36.11":
        raise ValueError("selected protoc-gen-go module differs from the pinned version")
    with tempfile.TemporaryDirectory() as temporary:
        output = Path(temporary)
        plugin = output / "protoc-gen-go"
        subprocess.run([str(go), "build", "-p=2", "-mod=readonly", "-trimpath", "-buildvcs=false",
                        "-o", str(plugin), "google.golang.org/protobuf/cmd/protoc-gen-go"],
                       cwd=MODULE, env=environment, check=True)
        command = [str(protoc), "-I", str(ROOT / "proto"),
                   "--plugin=protoc-gen-go=" + str(plugin), "--go_out=" + str(output),
                   "--go_opt=module=" + PREFIX]
        command += ["--go_opt=M" + name + "=" + PREFIX + "/gen/" + package
                    for name, package in FILES.items()]
        subprocess.run(command + sorted(FILES), cwd=ROOT, check=True)
        outputs = {"services/orchestration/gen/" + str(p.relative_to(output / "gen")): p.read_bytes()
                   for p in sorted((output / "gen").rglob("*.pb.go"))}
        inputs = ["proto/" + name for name in FILES]
        inputs += ["services/orchestration/go.mod", "services/orchestration/go.sum",
                   "scripts/protocol/regenerate_go.py", "scripts/protocol/provision_go.py"]
        manifest = {"schema": "celetixo/go-protocol-generation/v1", "go_version": VERSION,
                    "protoc_binary_sha256": BINARY_SHA256, "protoc_gen_go_version": "v1.36.11",
                    "inputs": {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sorted(inputs)},
                    "outputs": {p: hashlib.sha256(b).hexdigest() for p, b in outputs.items()}}
        outputs["proto/go-generation.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
        return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    outputs = generate(args.cache_dir, args.offline)
    extras = {str(p.relative_to(ROOT)) for p in (MODULE / "gen").rglob("*.go")} - set(outputs)
    if extras:
        raise ValueError("unrecognized generated Go files require explicit review")
    for name, content in outputs.items():
        target = ROOT / name
        if args.check:
            if not target.is_file() or target.read_bytes() != content:
                raise ValueError("generated Go binding differs: " + name)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
    print("canonical Go bindings " + ("verified" if args.check else "generated"))


if __name__ == "__main__":
    main()

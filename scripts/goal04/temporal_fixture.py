#!/usr/bin/env python3
"""Trusted operator fixture for the bounded local Temporal/PostgreSQL profile.

Preparation is explicit. Stop retains databases, credentials, network and image.
The fixture never grants Engine authority and never deletes retained state.
"""
import argparse
import fcntl
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path
import platform
import secrets
import socket
import ssl
import stat
import subprocess
import tarfile
import time

import nats_fixture as certificates
import postgres_fixture as pg

LABEL = "dev.celetixo.temporal-fixture-root"
SCHEMA = "celetixo/local-temporal-fixture/v1"
PORTS = {"frontend": 7233, "history": 7234, "matching": 7235, "worker": 7239}
SCOPE = {"organization_id": "fixture-organization", "workspace_id": "fixture-workspace",
         "repository_id": "fixture-repository"}
RUNTIME_FILES = {"ca.pem", "route-ca.pem", "frontend.pem", "frontend.key", "internode.pem",
                 "internode.key", "system-worker.pem", "system-worker.key", "identities.json",
                 "database.password", "server.yaml", "operator.json", "dynamic.yaml"}


def run(arguments, *, content=None, check=True, timeout=60):
    result = subprocess.run(arguments, input=content, capture_output=True, timeout=timeout)
    if check and result.returncode:
        # Vendor errors may contain connection configuration. Do not render them.
        raise RuntimeError("Temporal fixture command failed; inspect private operator logs")
    return result


def digest(path):
    if not path.is_absolute() or path.resolve() != path:
        raise RuntimeError("artifact must have an absolute canonical path")
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
        raise RuntimeError("artifact must be an owned regular file with one link")
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def namespace(scope):
    data = b"celetixo/temporal-namespace/v1"
    for key in ("organization_id", "workspace_id", "repository_id"):
        value = scope[key].encode()
        data += len(value).to_bytes(8, "big") + value
    return "celetixo-" + hashlib.sha256(data).hexdigest()


class Fixture:
    def __init__(self, root):
        if not root.is_absolute() or root.resolve() != root:
            raise RuntimeError("fixture root must be an absolute canonical path")
        self.root = root
        self.network = "celetixo-temporal-" + hashlib.sha256(str(root).encode()).hexdigest()[:12]
        self.name = self.network + "-server"
        self.database = pg.Fixture(root / "pg")

    def manifest(self):
        result = json.loads(pg.private_read(self.root / "manifest.json"))
        if result["schema"] != SCHEMA or result["root"] != str(self.root) or result["network"] != self.network:
            raise RuntimeError("retained fixture identity differs")
        for name in ("binary", "probe"):
            if digest(Path(result[name])) != result[name + "_sha256"]:
                raise RuntimeError("retained fixture artifact changed; preserve state for explicit upgrade")
        return result

    def inspect(self):
        result = run(["docker", "inspect", self.name], check=False)
        if result.returncode:
            if b"No such" in result.stderr:
                return None
            raise RuntimeError("container inspection failed")
        selected = json.loads(result.stdout)[0]
        if selected["Config"].get("Labels", {}).get(LABEL) != str(self.root):
            raise RuntimeError("container belongs to another operation")
        return selected

    def create_network(self):
        existing = run(["docker", "network", "inspect", self.network], check=False)
        if existing.returncode == 0:
            raise RuntimeError("unexpected retained network without a completed manifest")
        occupied = []
        for identifier in run(["docker", "network", "ls", "--quiet"]).stdout.decode().splitlines():
            info = json.loads(run(["docker", "network", "inspect", identifier]).stdout)[0]
            occupied += [ipaddress.ip_network(x["Subnet"]) for x in (info.get("IPAM", {}).get("Config") or [])
                         if x.get("Subnet") and ipaddress.ip_network(x["Subnet"]).version == 4]
        selected = next((ipaddress.ip_network(f"172.{n}.0.0/16") for n in range(16, 32)
                         if not any(ipaddress.ip_network(f"172.{n}.0.0/16").overlaps(x) for x in occupied)), None)
        if selected is None:
            raise RuntimeError("no unoccupied fixture subnet")
        run(["docker", "network", "create", "--internal", "--subnet", str(selected),
             "--label", LABEL + "=" + str(self.root), self.network])
        return str(selected.network_address + 2)

    def build_image(self, module_cache):
        license_path = module_cache / "go.temporal.io/server@v1.32.1/LICENSE"
        license_bytes = license_path.read_bytes()
        image = "celetixo-temporal-fixture-base:" + hashlib.sha256(license_bytes).hexdigest()[:16]
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w") as bundle:
            for name, data in [("Dockerfile", b'FROM scratch\nCOPY LICENSE /LICENSE\nENTRYPOINT ["/celetixo-temporal"]\n'),
                               ("LICENSE", license_bytes)]:
                member = tarfile.TarInfo(name)
                member.size, member.mode = len(data), 0o444
                bundle.addfile(member, io.BytesIO(data))
        run(["docker", "build", "--network=none", "--tag", image, "-"], content=archive.getvalue(), timeout=90)
        return image, json.loads(run(["docker", "image", "inspect", image]).stdout)[0]["Id"]

    def prepare(self, binary, probe, module_cache):
        if (self.root / "manifest.json").exists():
            return self.manifest()
        if set(x.name for x in self.root.iterdir()) - {"fixture.lock"}:
            raise RuntimeError("partial preparation retained; inspect before explicit recovery")
        hashes = {"binary_sha256": digest(binary), "probe_sha256": digest(probe)}
        for name in ("certs", "runtime", "migration", "clients", "logs", "build"):
            pg.private_directory(self.root / name)
        self.database.start()
        self.database.verify()
        for kind, role in (("migration", "temporal_migration"), ("runtime", "temporal_runtime")):
            password = secrets.token_hex(32)
            pg.private_write(self.root / kind / "database.password", password + "\n")
            self.database.secret_values.append(password)
            self.database.psql("CREATE ROLE " + role + " LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE "
                               "NOREPLICATION NOBYPASSRLS PASSWORD '" + password + "';", database="postgres")
        for database in ("temporal_workflows", "temporal_visibility"):
            self.database.psql("CREATE DATABASE " + database + " OWNER temporal_migration;", database="postgres")
            self.database.psql("REVOKE ALL ON DATABASE " + database + " FROM PUBLIC; GRANT CONNECT ON DATABASE "
                               + database + " TO temporal_migration,temporal_runtime;", database="postgres")
        address = self.create_network()
        certs = self.root / "certs"
        for issuer in ("ca", "route-ca"):
            run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
                 "-keyout", str(certs / (issuer + ".key")), "-out", str(certs / (issuer + ".pem")),
                 "-subj", "/CN=Celetixo Private Temporal " + issuer, "-days", "30", "-sha256"])
            for suffix in ("key", "pem"):
                (certs / (issuer + "." + suffix)).chmod(0o600)
        cert_tool = certificates.Fixture(self.root, self.root / "build")
        for name, server, issuer in [("frontend", True, "ca"), ("internode", True, "route-ca")]:
            cert_tool.certificate(name, server=server, addresses=[address], issuer=issuer)
        for name in ("system-worker", "operator", "client", "worker", "unmapped"):
            cert_tool.certificate(name)
        for name in RUNTIME_FILES & {p.name for p in certs.iterdir()}:
            pg.private_write(self.root / "runtime" / name, pg.private_read(certs / name))
        bindings = []
        for name, role in (("operator", 1), ("system-worker", 1), ("internode", 1), ("client", 2), ("worker", 3)):
            der = run(["openssl", "x509", "-in", str(certs / (name + ".pem")), "-outform", "DER"]).stdout
            bindings.append({"certificate_sha256": hashlib.sha256(der).hexdigest(), "principal": name,
                             "role": role, "scope": SCOPE if role != 1 else None,
                             "task_queues": ["engine"] if role != 1 else []})
        pg.private_write(self.root / "runtime/identities.json", json.dumps({"contract_version": 1, "bindings": bindings}))
        dynamic = {"frontend.enableTokenNamespaceEnforcement": [{"value": True}],
                   "frontend.enableServerVersionCheck": [{"value": False}],
                   "system.forceSearchAttributesCacheRefreshOnRead": [{"value": True}],
                   "frontend.rps": [{"value": 50}], "frontend.namespaceRPS": [{"value": 30}],
                   "history.persistenceMaxQPS": [{"value": 100}], "matching.persistenceMaxQPS": [{"value": 100}]}
        pg.private_write(self.root / "runtime/dynamic.yaml", json.dumps(dynamic))
        for kind in ("migration", "runtime"):
            self.write_server(kind, address)
        for name in ("operator", "client", "worker", "unmapped"):
            pg.private_write(self.root / "clients" / (name + ".json"), json.dumps({
                "contract_version": 1, "endpoint": address + ":7233", "server_name": "localhost",
                "ca_file": str(certs / "ca.pem"), "certificate": str(certs / (name + ".pem")),
                "private_key": str(certs / (name + ".key")), "scope": SCOPE, "task_queue": "engine"}))
        image, image_id = self.build_image(module_cache)
        manifest = {"schema": SCHEMA, "root": str(self.root), "network": self.network, "address": address,
                    "image": image, "image_id": image_id, "binary": str(binary), "probe": str(probe),
                    "scope": SCOPE, "namespace": namespace(SCOPE), **hashes}
        # Explicit isolated migration process; serving never runs DDL.
        log = self.root / "logs/migration.raw.log"
        descriptor = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            result = subprocess.run(["docker", "run", "--rm", *self.common(manifest), "--mount",
                                     f"type=bind,src={self.root}/migration,dst=/operator,readonly", image_id,
                                     "migrate", "--config", "/operator/operator.json"],
                                    stdout=output, stderr=subprocess.STDOUT, timeout=300)
        if result.returncode:
            raise RuntimeError("schema migration failed; private diagnostics and data retained")
        self.grant_runtime()
        pg.private_write(self.root / "manifest.json", json.dumps(manifest, indent=2) + "\n")
        return manifest

    def write_server(self, kind, address):
        stores = {}
        for key, database in (("workflow", "temporal_workflows"), ("visibility", "temporal_visibility")):
            stores[key] = {"sql": {"pluginName": "postgres12_pgx", "user": "temporal_" + kind,
                "databaseName": database, "connectAddr": "localhost:5432", "connectProtocol": "unix",
                "maxConns": 4, "maxIdleConns": 2, "maxConnLifetime": "1m", "connectAttributes": {
                    "host": "/run/postgresql", "port": "5432", "sslmode": "disable", "require_auth": "scram-sha-256",
                    "connect_timeout": "3", "statement_timeout": "5000", "lock_timeout": "5000"}}}
        def client(issuer):
            return {"serverName": "localhost", "rootCaFiles": ["/operator/" + issuer + ".pem"]}
        def group(name, issuer):
            return {"server": {"certFile": "/operator/" + name + ".pem", "keyFile": "/operator/" + name + ".key",
                "clientCaFiles": ["/operator/" + issuer + ".pem"], "requireClientAuth": True}, "client": client(issuer)}
        server = {"log": {"stdout": True, "level": "warn", "format": "json"},
            "persistence": {"defaultStore": "workflow", "visibilityStore": "visibility", "numHistoryShards": 4, "datastores": stores},
            "global": {"membership": {"maxJoinDuration": "30s", "broadcastAddress": address}, "pprof": {"port": 0},
                "tls": {"frontend": group("frontend", "ca"), "internode": group("internode", "route-ca"),
                    "systemWorker": {"certFile": "/operator/system-worker.pem", "keyFile": "/operator/system-worker.key", "client": client("ca")}}},
            "services": {name: {"rpc": {"grpcPort": port, "bindOnIP": address}} for name, port in PORTS.items()},
            "clusterMetadata": {"enableGlobalNamespace": False, "failoverVersionIncrement": 10, "masterClusterName": "local",
                "currentClusterName": "local", "clusterInformation": {"local": {"enabled": True, "initialFailoverVersion": 1,
                    "rpcAddress": address + ":7233"}}},
            "publicClient": {"hostPort": address + ":7233", "forceTLSConfig": "frontend"}, "dcRedirectionPolicy": {"policy": "noop"},
            "archival": {key: {"state": "disabled", "enableRead": False} for key in ("history", "visibility")},
            "namespaceDefaults": {"archival": {key: {"state": "disabled"} for key in ("history", "visibility")}},
            "dynamicConfigClient": {"filepath": "/operator/dynamic.yaml", "pollInterval": "10s"}}
        operator = {"contract_version": 1, "server_file": "/operator/server.yaml", "identity_file": "/operator/identities.json",
            "database_password_files": {key: "/operator/database.password" for key in stores},
            "static_hosts": {name: {"self": f"{address}:{port}", "all": [f"{address}:{port}"]} for name, port in PORTS.items()}}
        pg.private_write(self.root / kind / "server.yaml", json.dumps(server, indent=2))
        pg.private_write(self.root / kind / "operator.json", json.dumps(operator, indent=2))

    def grant_runtime(self):
        for database in ("temporal_workflows", "temporal_visibility"):
            self.database.psql("REVOKE CREATE ON SCHEMA public FROM PUBLIC; GRANT USAGE ON SCHEMA public TO temporal_runtime; "
                               "GRANT SELECT ON public.schema_version,public.schema_update_history TO temporal_runtime;", database=database)
            tables = self.database.psql("SELECT tablename FROM pg_tables WHERE schemaname='public' AND "
                "tablename NOT IN ('schema_version','schema_update_history') ORDER BY tablename;", database=database).stdout.strip().splitlines()
            if not tables or any(not name or any(ch not in "abcdefghijklmnopqrstuvwxyz0123456789_" for ch in name) for name in tables):
                raise RuntimeError("unexpected vendor table identifier")
            self.database.psql("GRANT SELECT,INSERT,UPDATE,DELETE ON " + ",".join("public." + name for name in tables)
                               + " TO temporal_runtime; GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO temporal_runtime;", database=database)

    def common(self, manifest):
        return ["--mount", f'type=bind,src={manifest["binary"]},dst=/celetixo-temporal,readonly',
            "--network", self.network, "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop=ALL",
            "--security-opt=no-new-privileges", "--read-only", "--cpus=1", "--memory=1g", "--pids-limit=128",
            "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=32m,mode=1777", "--env", "USER=celetixo-temporal",
            "--env", "GOMEMLIMIT=768MiB", "--env", "GOMAXPROCS=2", "--log-opt", "max-size=2m", "--log-opt", "max-file=2",
            "--mount", f"type=bind,src={self.root}/pg/socket,dst=/run/postgresql,readonly"]

    def probe(self, manifest, mode):
        identity = "operator" if mode in ("setup", "health") else mode
        result = run([manifest["probe"], mode, "--config", str(self.root / "clients" / (identity + ".json"))], timeout=50)
        return json.loads(result.stdout)

    def start(self):
        manifest = self.manifest()
        self.database.start()
        if self.inspect() is None:
            run(["docker", "create", "--name", self.name, "--label", LABEL + "=" + str(self.root),
                "--ip", manifest["address"], *self.common(manifest), "--mount",
                f"type=bind,src={self.root}/runtime,dst=/operator,readonly", manifest["image_id"],
                "serve", "--config", "/operator/operator.json"])
        self.runtime_profile(manifest)
        run(["docker", "start", self.name])
        self.probe(manifest, "setup")
        return {"ready": True, "namespace": manifest["namespace"], "endpoint": manifest["address"] + ":7233"}

    def runtime_profile(self, manifest):
        item = self.inspect()
        if item is None:
            raise RuntimeError("missing server container")
        host = item["HostConfig"]
        if (item["Image"] != manifest["image_id"] or item["Config"]["User"] != f"{os.getuid()}:{os.getgid()}"
                or host["NetworkMode"] != self.network or not host["ReadonlyRootfs"] or host["Memory"] != 1024**3
                or host["PidsLimit"] != 128 or host["NanoCpus"] != 1000000000 or host["PortBindings"]
                or "ALL" not in host["CapDrop"] or not any(x.startswith("no-new-privileges") for x in host["SecurityOpt"])):
            raise RuntimeError("server differs from bounded runtime profile")
        mounts = {x["Destination"]: x for x in item["Mounts"] if x["Type"] == "bind"}
        expected = {"/operator": str(self.root / "runtime"), "/run/postgresql": str(self.root / "pg/socket"),
                    "/celetixo-temporal": manifest["binary"]}
        if set(mounts) != set(expected) or any(mounts[k]["Source"] != v or mounts[k]["RW"] for k, v in expected.items()):
            raise RuntimeError("unexpected host mount")
        if {x.name for x in (self.root / "runtime").iterdir()} != RUNTIME_FILES:
            raise RuntimeError("server mount exposes unexpected files")
        network = json.loads(run(["docker", "network", "inspect", self.network]).stdout)[0]
        if not network["Internal"] or network.get("Labels", {}).get(LABEL) != str(self.root):
            raise RuntimeError("network ownership or egress policy changed")

    def verify(self):
        manifest = self.manifest()
        self.runtime_profile(manifest)
        self.database.verify()
        checks = [self.probe(manifest, mode) for mode in ("health", "worker", "client", "unmapped")]
        def context(identity="operator", issuer="ca", maximum=None):
            result = ssl.create_default_context(cafile=str(self.root / "certs" / (issuer + ".pem")))
            result.minimum_version = ssl.TLSVersion.TLSv1_3 if maximum is None else maximum
            if maximum is not None:
                result.maximum_version = maximum
            if identity:
                result.load_cert_chain(str(self.root / "certs" / (identity + ".pem")), str(self.root / "certs" / (identity + ".key")))
            result.set_alpn_protocols(["h2"])
            return result
        denied = []
        for label, ctx, hostname, port in [
            ("wrong_ca", context(issuer="route-ca"), "localhost", 7233),
            ("wrong_hostname", context(), "wrong.invalid", 7233),
            ("missing_client_certificate", context(identity=None), "localhost", 7233),
            ("tls12", context(maximum=ssl.TLSVersion.TLSv1_2), "localhost", 7233),
            ("client_certificate_at_internode", context(issuer="route-ca"), "localhost", 7234)]:
            try:
                with socket.create_connection((manifest["address"], port), timeout=3) as peer:
                    with ctx.wrap_socket(peer, server_hostname=hostname) as secure:
                        secure.sendall(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
                        data = secure.recv(65536)
                        if not data:
                            denied.append(label)
                            continue
            except ssl.SSLError:
                denied.append(label)
                continue
            raise RuntimeError("TLS negative probe did not prove rejection: " + label)
        with socket.create_connection((manifest["address"], 7234), timeout=3) as peer:
            with context("internode", "route-ca").wrap_socket(peer, server_hostname="localhost") as secure:
                if secure.version() != "TLSv1.3":
                    raise RuntimeError("internode TLS positive control failed")
        return {"schema": SCHEMA, "running": True, "server_version": "1.32.1", "postgres": "17.9",
                "namespace": manifest["namespace"], "internal_network": True, "host_ports_published": False,
                "separate_internode_ca": True, "server_mount_has_only_own_keys": True,
                "rpc_checks": checks, "tls_denials": denied, "claim": "Coordination bootstrap only; no Engine workflow acceptance"}

    def stop(self):
        item = self.inspect()
        if item and item["State"]["Running"]:
            run(["docker", "stop", "--time=30", self.name], timeout=40)
        self.database.stop()
        return {"stopped": True, "all_data_retained": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "start", "stop", "status", "verify"))
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--probe", type=Path)
    parser.add_argument("--module-cache", type=Path, default=Path(os.environ.get("GOMODCACHE", str(Path.home() / "go/pkg/mod"))))
    args = parser.parse_args()
    if os.getuid() == 0 or platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        raise RuntimeError("fixture requires a non-root Linux x86_64 operator")
    if not os.environ.get("DOCKER_HOST", "unix://local").startswith("unix://"):
        raise RuntimeError("fixture requires local Unix-socket Docker")
    endpoint = run(["docker", "context", "inspect", "--format", "{{.Endpoints.docker.Host}}"] ).stdout.strip()
    if not endpoint.startswith(b"unix://"):
        raise RuntimeError("fixture requires local Unix-socket Docker context")
    fixture = Fixture(args.root)
    pg.private_directory(args.root)
    lock = args.root / "fixture.lock"
    if not lock.exists():
        pg.private_write(lock, "operator fixture lock\n")
    pg.private_read(lock)
    with lock.open("r+") as descriptor:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.action == "prepare":
            if args.binary is None or args.probe is None:
                raise RuntimeError("explicit preparation requires host and probe artifacts")
            result = fixture.prepare(args.binary, args.probe, args.module_cache)
            result = {"prepared": True, "namespace": result["namespace"], "manifest": str(args.root / "manifest.json")}
        elif args.action == "status":
            result = {"running": bool((fixture.inspect() or {}).get("State", {}).get("Running"))}
        else:
            result = getattr(fixture, args.action)()
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()

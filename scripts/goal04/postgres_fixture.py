#!/usr/bin/env python3
"""Operator-owned local PostgreSQL fixture; never executes repository programs."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import stat
import struct
import subprocess
import sys
import time


REPO_DIGEST = "postgres@sha256:c7526c0f6c3f30260a563d7bcf8ad778effac59a44f8ffa86678c35418338609"
DEFAULT_ROOT = Path.home() / ".local/share/celetixo-goal04/service-fixtures/postgres17"
DATABASE = "celetixo"
ROLES = {
    "admin": "celetixo_fixture_admin",
    "migration": "celetixo_migration",
    "runtime": "celetixo_runtime",
    "runtime_other": "celetixo_runtime_other",
}
LABEL = "dev.celetixo.fixture-root"
DATA_BUDGET = 1024 * 1024 * 1024
PHYSICAL_RESERVE = 512 * 1024 * 1024


class FixtureError(Exception):
    pass


def private_directory(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise FixtureError(f"fixture directory has an unsafe owner or type: {path}")
    if stat.S_IMODE(info.st_mode) != 0o700:
        raise FixtureError(f"fixture directory must have mode 0700: {path}")


def private_read(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise FixtureError(f"private file has an unsafe owner or type: {path}")
        if stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1:
            raise FixtureError(f"private file must be unlinked elsewhere and mode 0600: {path}")
        if info.st_size > 65536:
            raise FixtureError("private configuration exceeds its size budget")
        with os.fdopen(os.dup(fd), encoding="utf-8") as stream:
            return stream.read()
    finally:
        os.close(fd)


def private_write(path, value):
    if path.exists() or path.is_symlink():
        private_read(path)
    temporary = path.with_name(path.name + ".pending-" + secrets.token_hex(8))
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


class Fixture:
    def __init__(self, root):
        if not root.is_absolute() or root.resolve() != root:
            raise FixtureError("fixture root must be an absolute, nonsymlink path")
        self.root = root
        self.name = "celetixo-goal04-pg17-" + hashlib.sha256(str(root).encode()).hexdigest()[:12]
        self.secret_values = []
        self.image = None

    def cached_image(self):
        if self.image is None:
            result = self.run(["docker", "image", "inspect", REPO_DIGEST])
            image = json.loads(result.stdout)[0]
            if (REPO_DIGEST not in image.get("RepoDigests", [])
                    or image.get("Os") != "linux" or image.get("Architecture") != "amd64"
                    or not image.get("Id", "").startswith("sha256:")):
                raise FixtureError("cached image does not match the reviewed repository digest/platform")
            self.image = image
        return self.image

    def run(self, args, *, sql=None, check=True, timeout=45):
        result = subprocess.run(
            args,
            input=sql,
            text=True,
            capture_output=True,
            timeout=timeout,
            env={**os.environ, "DOCKER_CLI_HINTS": "false"},
        )
        if check and result.returncode:
            diagnostic = result.stderr[-3000:]
            for value in self.secret_values:
                diagnostic = diagnostic.replace(value, "[redacted]")
            raise FixtureError(f"fixture operation failed ({result.returncode}): {diagnostic}")
        return result

    def inspect(self):
        result = self.run(["docker", "container", "inspect", self.name], check=False)
        if result.returncode:
            if "No such" in result.stderr:
                return None
            raise FixtureError("cannot inspect the fixture container")
        item = json.loads(result.stdout)[0]
        if item["Config"].get("Labels", {}).get(LABEL) != str(self.root):
            raise FixtureError("container name is occupied by a different owner")
        if item["Image"] != self.cached_image()["Id"] or item["Config"]["User"] != f"{os.getuid()}:{os.getgid()}":
            raise FixtureError("existing fixture container has different image or user")
        if item["HostConfig"]["NetworkMode"] != "none" or item["HostConfig"]["PortBindings"]:
            raise FixtureError("existing fixture container unexpectedly exposes a network")
        return item

    def budget(self):
        total = 0
        files = 0
        for directory, subdirs, names in os.walk(self.root / "data", followlinks=False):
            for name in subdirs + names:
                path = Path(directory) / name
                try:
                    item = path.lstat()
                except FileNotFoundError:
                    continue  # PostgreSQL can atomically replace temporary files.
                if stat.S_ISLNK(item.st_mode):
                    raise FixtureError("unexpected symlink in fixture PGDATA")
                if stat.S_ISREG(item.st_mode):
                    total += item.st_size
                    files += 1
                if total > DATA_BUDGET or files > 20000:
                    raise FixtureError("fixture data exceeds its 1 GiB / 20000-file guard")
        backing_free = shutil.disk_usage("/mnt/c" if Path("/mnt/c").is_dir() else self.root).free
        if backing_free < PHYSICAL_RESERVE:
            raise FixtureError("physical backing filesystem has less than 512 MiB free")
        return {"data_bytes": total, "data_files": files, "data_budget_bytes": DATA_BUDGET,
                "backing_free_bytes": backing_free, "guard_is_hard_quota": False}

    def prepare(self):
        private_directory(self.root)
        for name in ["secrets", "config", "data", "socket"]:
            private_directory(self.root / name)
        host = str(self.root / "socket")
        if len(os.fsencode(host + "/.s.PGSQL.5432")) >= 108:
            raise FixtureError("fixture Unix-socket path exceeds the Linux path budget")
        image = self.cached_image()
        roles = {}
        for key, user in ROLES.items():
            password_file = self.root / "secrets" / (key + ".password")
            if not password_file.exists():
                private_write(password_file, secrets.token_hex(32) + "\n")
            password = private_read(password_file).strip()
            if len(password) != 64 or any(char not in "0123456789abcdef" for char in password):
                raise FixtureError("invalid fixture-generated password format")
            self.secret_values.append(password)
            private_write(self.root / "secrets" / (key + ".pgpass"),
                          f"/var/run/celetixo:5432:*:{user}:{password}\n")
            roles[key] = {"user": user, "password_file": str(password_file)}
        private_write(self.root / "secrets/invalid.pgpass",
                      f"/var/run/celetixo:5432:*:{ROLES['runtime']}:deliberately-invalid\n")
        private_write(self.root / "config/passwd",
                      f"root:x:0:0:root:/root:/sbin/nologin\nceletixopg:x:{os.getuid()}:{os.getgid()}:Celetixo fixture:/var/lib/postgresql:/sbin/nologin\n")
        private_write(self.root / "config/group", f"root:x:0:\nceletixopg:x:{os.getgid()}:\n")
        private_write(self.root / "config/pg_hba.conf",
                      "local all all scram-sha-256\nhost all all 0.0.0.0/0 reject\nhost all all ::0/0 reject\n")
        private_write(self.root / "config/pg_ident.conf", "# No peer identity mapping.\n")
        private_write(self.root / "config/postgresql.conf", """listen_addresses = ''
port = 5432
unix_socket_directories = '/var/run/celetixo'
unix_socket_permissions = 0700
password_encryption = 'scram-sha-256'
fsync = on
full_page_writes = on
synchronous_commit = on
wal_level = replica
max_wal_size = '128MB'
min_wal_size = '32MB'
checkpoint_timeout = '5min'
shared_buffers = '32MB'
work_mem = '2MB'
maintenance_work_mem = '16MB'
temp_file_limit = '64MB'
max_connections = 20
max_worker_processes = 4
max_parallel_workers = 2
log_destination = 'stderr'
logging_collector = off
log_statement = 'none'
log_min_error_statement = 'panic'
log_error_verbosity = 'terse'
log_connections = off
log_disconnections = off
""")
        manifest = {"schema_version": 1, "host": host, "port": 5432, "database": DATABASE,
                    "roles": roles, "container_name": self.name, "image_id": image["Id"],
                    "repository_digest": REPO_DIGEST}
        private_write(self.root / "connection.json", json.dumps(manifest, indent=2) + "\n")
        return manifest

    def common(self):
        args = ["--pull", "never", "--platform", "linux/amd64", "--network", "none", "--read-only", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--user", f"{os.getuid()}:{os.getgid()}",
                "--cpus", "1", "--memory", "384m", "--memory-swap", "384m",
                "--pids-limit", "96", "--shm-size", "32m", "--ulimit", "nofile=4096:4096",
                "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777",
                "--log-driver", "local", "--log-opt", "max-size=2m", "--log-opt", "max-file=2",
                "--label", f"{LABEL}={self.root}"]
        for source, destination, readonly in [
            ("data", "/var/lib/postgresql/data", False),
            ("socket", "/var/run/celetixo", False),
            ("secrets", "/run/secrets", True),
            ("config", "/etc/celetixo", True),
            ("config/passwd", "/etc/passwd", True),
            ("config/group", "/etc/group", True),
        ]:
            mount = f"type=bind,src={self.root / source},dst={destination}"
            if readonly:
                mount += ",readonly"
            args += ["--mount", mount]
        return args

    def psql(self, sql, key="admin", database=DATABASE, check=True, invalid=False):
        name = "invalid" if invalid else key
        return self.run(["docker", "exec", "--interactive", "--env", f"PGPASSFILE=/run/secrets/{name}.pgpass",
                         self.name, "/usr/local/bin/psql", "--host=/var/run/celetixo", "--port=5432",
                         "--username=" + ROLES[key], "--dbname=" + database, "--no-password",
                         "--no-psqlrc", "--set=ON_ERROR_STOP=1", "--quiet", "--tuples-only", "--no-align"],
                        sql=sql, check=check)

    def wait(self):
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            item = self.inspect()
            if not item or not item["State"]["Running"]:
                raise FixtureError("fixture PostgreSQL process exited; inspect its bounded Docker log")
            if self.psql("SELECT 1;", database="postgres", check=False).returncode == 0:
                return
            time.sleep(0.2)
        raise FixtureError("PostgreSQL readiness deadline exceeded")

    def start(self):
        manifest = self.prepare()
        self.budget()
        item = self.inspect()
        if not (self.root / "data/PG_VERSION").exists():
            if any((self.root / "data").iterdir()):
                raise FixtureError("incomplete initial PGDATA requires operator review; never deleted automatically")
            self.run(["docker", "run", "--rm", *self.common(), "--entrypoint", "/usr/local/bin/initdb",
                      REPO_DIGEST, "-D", "/var/lib/postgresql/data", "--username=" + ROLES["admin"],
                      "--pwfile=/run/secrets/admin.password", "--auth-local=scram-sha-256",
                      "--auth-host=reject", "--encoding=UTF8", "--no-locale", "--data-checksums"], timeout=90)
        if private_read(self.root / "data/PG_VERSION").strip() != "17":
            raise FixtureError("unexpected PostgreSQL data major version")
        if item is None:
            self.run(["docker", "create", "--name", self.name, *self.common(), "--entrypoint",
                      "/usr/local/bin/postgres", REPO_DIGEST, "-D", "/var/lib/postgresql/data",
                      "-c", "config_file=/etc/celetixo/postgresql.conf", "-c", "hba_file=/etc/celetixo/pg_hba.conf",
                      "-c", "ident_file=/etc/celetixo/pg_ident.conf"])
            item = self.inspect()
        if not item["State"]["Running"]:
            self.run(["docker", "start", self.name])
        self.wait()
        for key in ["migration", "runtime", "runtime_other"]:
            role = ROLES[key]
            password = private_read(self.root / "secrets" / (key + ".password")).strip()
            exists = self.psql(f"SELECT count(*) FROM pg_roles WHERE rolname = '{role}';", database="postgres").stdout.strip()
            if exists == "0":
                self.psql(f"CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{password}';", database="postgres")
        exists = self.psql(f"SELECT count(*) FROM pg_database WHERE datname = '{DATABASE}';", database="postgres").stdout.strip()
        if exists == "0":
            self.psql(f"CREATE DATABASE {DATABASE} OWNER {ROLES['migration']};", database="postgres")
        self.psql(f"REVOKE ALL ON DATABASE {DATABASE} FROM PUBLIC; GRANT CONNECT ON DATABASE {DATABASE} TO {ROLES['migration']}, {ROLES['runtime']}, {ROLES['runtime_other']}; REVOKE ALL ON SCHEMA public FROM PUBLIC;")
        return {"connection_file": str(self.root / "connection.json"), **self.verify(),
                "roles": {key: value["user"] for key, value in manifest["roles"].items()}}

    def stop(self):
        item = self.inspect()
        if item and item["State"]["Running"]:
            self.run(["docker", "stop", "--time", "30", self.name])
        return {"container_name": self.name, "running": False, "data_preserved": True}

    def status(self):
        item = self.inspect()
        return {"container_name": self.name, "exists": item is not None,
                "running": bool(item and item["State"]["Running"]),
                "connection_file": str(self.root / "connection.json"), **self.budget()}

    def verify(self):
        item = self.inspect()
        if not item or not item["State"]["Running"]:
            raise FixtureError("fixture is not running")
        controls = item["HostConfig"]
        expected_controls = {"ReadonlyRootfs": True, "Privileged": False,
                             "NanoCpus": 1000000000, "Memory": 384 * 1024 * 1024,
                             "MemorySwap": 384 * 1024 * 1024, "PidsLimit": 96,
                             "ShmSize": 32 * 1024 * 1024}
        if any(controls.get(key) != value for key, value in expected_controls.items()):
            raise FixtureError("container resource or filesystem controls differ from the fixture profile")
        if controls.get("CapDrop") != ["ALL"] or controls.get("SecurityOpt") != ["no-new-privileges"]:
            raise FixtureError("container privilege controls differ from the fixture profile")
        if controls.get("LogConfig") != {"Type": "local", "Config": {"max-size": "2m", "max-file": "2"}}:
            raise FixtureError("container log bounds differ from the fixture profile")
        mounts = {mount["Destination"]: mount for mount in item["Mounts"]}
        for destination in ["/run/secrets", "/etc/celetixo", "/etc/passwd", "/etc/group"]:
            if destination not in mounts or mounts[destination]["RW"]:
                raise FixtureError("private container configuration is not mounted read-only")
        expected = {"fsync": "on", "full_page_writes": "on", "synchronous_commit": "on",
                    "listen_addresses": "", "password_encryption": "scram-sha-256",
                    "data_checksums": "on"}
        settings = {}
        for key, value in expected.items():
            actual = self.psql("SHOW " + key + ";").stdout.strip()
            if actual != value:
                raise FixtureError("PostgreSQL safety setting mismatch: " + key)
            settings[key] = actual
        for key in ROLES:
            actual = self.psql("SELECT session_user;", key=key).stdout.strip()
            if actual != ROLES[key]:
                raise FixtureError("role authentication returned a different session identity")
        for key in ["runtime", "runtime_other"]:
            escalated = self.psql("SELECT pg_has_role(session_user, 'celetixo_migration', 'MEMBER') OR has_database_privilege(session_user, current_database(), 'CREATE');", key=key).stdout.strip()
            if escalated != "f":
                raise FixtureError("runtime role can assume migration authority or create schemas")
        rejected = self.psql("SELECT 1;", key="runtime", check=False, invalid=True)
        if rejected.returncode == 0 or "password authentication failed" not in rejected.stderr:
            raise FixtureError("invalid password was not rejected through SCRAM")
        # Independently inspect the host-visible socket authentication challenge.
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(5)
            connection.connect(str(self.root / "socket/.s.PGSQL.5432"))
            body = struct.pack("!I", 196608) + b"user\0" + ROLES["runtime"].encode() + b"\0database\0" + DATABASE.encode() + b"\0\0"
            connection.sendall(struct.pack("!I", len(body) + 4) + body)
            def receive_exact(length):
                answer = bytearray()
                while len(answer) < length:
                    piece = connection.recv(length - len(answer))
                    if not piece:
                        raise FixtureError("host Unix socket closed during authentication")
                    answer.extend(piece)
                return bytes(answer)

            header = receive_exact(5)
            length = struct.unpack("!I", header[1:])[0]
            if header[:1] != b"R" or not 8 <= length <= 1024:
                raise FixtureError("host Unix socket returned an invalid authentication message")
            answer = receive_exact(length - 4)
            if struct.unpack("!I", answer[:4])[0] != 10 or b"SCRAM-SHA-256\0" not in answer:
                raise FixtureError("host Unix socket did not require SCRAM authentication")
        version = self.psql("SELECT version();").stdout.strip()
        roles = self.psql("SELECT rolname || '|' || rolsuper || '|' || rolcreatedb || '|' || rolcreaterole || '|' || rolreplication || '|' || rolbypassrls FROM pg_roles WHERE rolname IN ('celetixo_migration','celetixo_runtime','celetixo_runtime_other') ORDER BY rolname;").stdout.strip().splitlines()
        if any(line.split("|")[1:] != ["false"] * 5 for line in roles) or len(roles) != 3:
            raise FixtureError("application roles possess unexpected administrative privileges")
        return {"container_name": self.name, "running": True, "version": version,
                "settings": settings, "scram_rejects_invalid_password": True,
                "host_socket_requires_scram": True, "application_roles_are_nonprivileged": True,
                "network_mode": "none", "published_ports": [], **self.budget()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["start", "status", "verify", "stop", "restart"])
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    if os.getuid() == 0:
        raise FixtureError("run the operator fixture as a non-root user")
    if os.environ.get("DOCKER_HOST", "unix://local").startswith("unix://") is False:
        raise FixtureError("the fixture requires a local Unix-socket Docker endpoint")
    fixture = Fixture(args.root)
    endpoint = fixture.run(["docker", "context", "inspect", "--format", "{{.Endpoints.docker.Host}}"]).stdout.strip()
    if not endpoint.startswith("unix://"):
        raise FixtureError("the fixture requires a local Unix-socket Docker context")
    private_directory(fixture.root)
    lock = os.open(fixture.root / "operator.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.operation == "restart":
            fixture.stop()
            result = fixture.start()
        else:
            result = getattr(fixture, args.operation)()
        print(json.dumps(result, indent=2))
    finally:
        os.close(lock)


if __name__ == "__main__":
    try:
        main()
    except (FixtureError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"PostgreSQL fixture error: {error}", file=sys.stderr)
        sys.exit(1)

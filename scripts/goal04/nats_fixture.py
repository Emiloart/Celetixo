#!/usr/bin/env python3
"""Operator-owned, bounded three-node TLS NATS fixture; stop preserves all data."""
import argparse
import hashlib
import io
import ipaddress
import json
import os
import platform
from pathlib import Path
import shutil
import socket
import ssl
import stat
import subprocess
import tarfile
import time
import urllib.request

VERSION = "2.15.3"
BINARY_SHA256 = "c1a56aca1f0210120dd816a345a7da7561ad0adee9bcac59e1be5d7d111031f2"
LICENSE_SHA256 = "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4"
ARCHIVE_SHA256 = "32e18c1e78d7ef4f86e483070eef3abb2647c3d4d67e29668077c7ad81da0797"
LABEL = "dev.celetixo.nats-fixture-root"


def run(args, *, content=None, check=True, timeout=45):
    result = subprocess.run(args, input=content, capture_output=True, timeout=timeout)
    if check and result.returncode:
        # Arguments contain paths/selectors only; no secret bytes are printed.
        raise RuntimeError("fixture command failed: " + result.stderr[-2000:].decode(errors="replace"))
    return result


def directory(path):
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise RuntimeError("fixture directory must be private, owned and nonsymlink")


def private(path, content):
    if path.exists() or path.is_symlink():
        raise RuntimeError("refusing to overwrite retained fixture input")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())


def provision_tools(path):
    directory(path)
    targets = {"nats-server":BINARY_SHA256,"LICENSE":LICENSE_SHA256}
    if any((path / name).exists() for name in targets):
        for name,digest in targets.items():
            selected = path / name
            if not selected.is_file() or selected.is_symlink() or hashlib.sha256(selected.read_bytes()).hexdigest()!=digest:
                raise RuntimeError("existing NATS tool cache differs from pinned release")
        return {"cached":True,"version":VERSION,"binary_sha256":BINARY_SHA256}
    url="https://github.com/nats-io/nats-server/releases/download/v"+VERSION+"/nats-server-v"+VERSION+"-linux-amd64.tar.gz"
    request=urllib.request.Request(url,headers={"User-Agent":"Celetixo-operator-fixture"})
    with urllib.request.urlopen(request,timeout=30) as response:
        archive=response.read(16*1024**2+1)
    if len(archive)>16*1024**2 or hashlib.sha256(archive).hexdigest()!=ARCHIVE_SHA256:
        raise RuntimeError("NATS archive failed pinned digest/length verification")
    with tarfile.open(fileobj=io.BytesIO(archive),mode="r:gz") as bundle:
        extracted={}
        for name,digest in targets.items():
            item=bundle.getmember("nats-server-v"+VERSION+"-linux-amd64/"+name)
            if not item.isfile() or item.size>32*1024**2:
                raise RuntimeError("unexpected bounded NATS archive member")
            content=bundle.extractfile(item).read()
            if hashlib.sha256(content).hexdigest()!=digest:
                raise RuntimeError("NATS executable/license differs from pinned bytes")
            extracted[name]=content
    for name,content in extracted.items():
        private(path / name,content)
    (path / "nats-server").chmod(0o500)
    return {"cached":False,"version":VERSION,"archive_sha256":ARCHIVE_SHA256,"binary_sha256":BINARY_SHA256,"license_retained":True}


class Fixture:
    def __init__(self, root, tools):
        if not root.is_absolute() or root.resolve() != root:
            raise RuntimeError("fixture root must be an absolute nonsymlink path")
        self.root, self.tools = root, tools
        self.suffix = hashlib.sha256(str(root).encode()).hexdigest()[:12]
        self.network = "celetixo-events-" + self.suffix
        self.image = "celetixo-nats-fixture:" + BINARY_SHA256[:16]

    def name(self, node):
        return self.network + "-" + node

    def inspect(self, node):
        result = run(["docker", "inspect", self.name(node)], check=False)
        if result.returncode:
            if b"No such" in result.stderr:
                return None
            raise RuntimeError("container inspection failed")
        item = json.loads(result.stdout)[0]
        if item["Config"].get("Labels", {}).get(LABEL) != str(self.root):
            raise RuntimeError("container is owned by another operation")
        if item["Config"]["User"] != f"{os.getuid()}:{os.getgid()}":
            raise RuntimeError("unexpected fixture container identity")
        if item["HostConfig"]["NetworkMode"] != self.network:
            raise RuntimeError("unexpected fixture network")
        return item

    def certificate(self, name, server=False, addresses=(), issuer="ca"):
        certs = self.root / "certs"
        run(["openssl", "req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
             "-keyout", str(certs / (name + ".key")), "-out", str(certs / (name + ".csr")), "-subj", "/CN=" + name])
        extensions = "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n"
        extensions += "extendedKeyUsage=" + ("serverAuth,clientAuth" if server else "clientAuth") + "\n"
        extensions += "subjectAltName=" + ("DNS:n1,DNS:n2,DNS:n3,DNS:localhost,IP:127.0.0.1" + "".join(",IP:"+address for address in addresses) if server else "DNS:" + name) + "\n"
        private(certs / (name + ".ext"), extensions.encode())
        run(["openssl", "x509", "-req", "-in", str(certs / (name + ".csr")), "-CA", str(certs / (issuer+".pem")),
             "-CAkey", str(certs / (issuer+".key")), "-CAcreateserial", "-out", str(certs / (name + ".pem")),
             "-days", "30", "-sha256", "-extfile", str(certs / (name + ".ext"))])
        for path in certs.glob(name + ".*"):
            path.chmod(0o600)

    def prepare(self):
        directory(self.root)
        if (self.root / "manifest.json").exists():
            return json.loads((self.root / "manifest.json").read_text())
        if any(self.root.iterdir()):
            raise RuntimeError("incomplete preparation retained; inspect before retrying")
        if shutil.disk_usage("/mnt/c" if Path("/mnt/c").exists() else self.root).free < 1024**3:
            raise RuntimeError("fixture requires at least 1 GiB of backing reserve")
        binary = (self.tools / "nats-server").read_bytes()
        license_bytes = (self.tools / "LICENSE").read_bytes()
        if hashlib.sha256(binary).hexdigest() != BINARY_SHA256 or hashlib.sha256(license_bytes).hexdigest() != LICENSE_SHA256:
            raise RuntimeError("operator tool bytes differ from pinned official release")
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w") as bundle:
            for name, content, mode in [("Dockerfile", b"FROM scratch\nCOPY nats-server /nats-server\nCOPY LICENSE /LICENSE\nENTRYPOINT [\"/nats-server\"]\n", 0o644),
                                         ("nats-server", binary, 0o555), ("LICENSE", license_bytes, 0o444)]:
                item = tarfile.TarInfo(name)
                item.size, item.mode = len(content), mode
                bundle.addfile(item, io.BytesIO(content))
        run(["docker", "build", "--network=none", "--tag", self.image, "-"], content=archive.getvalue(), timeout=90)
        directory(self.root / "certs")
        for issuer in ["ca","route-ca"]:
            run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
                 "-keyout", str(self.root / ("certs/"+issuer+".key")), "-out", str(self.root / ("certs/"+issuer+".pem")),
                 "-subj", "/CN=Celetixo Private Event Fixture "+issuer, "-days", "30", "-sha256"])
        for path in (self.root / "certs").iterdir():
            path.chmod(0o600)
        # An explicit nonoverlapping subnet permits pinned internal addresses.
        networks = [json.loads(line) for line in run(["docker", "network", "ls", "--format", "json"]).stdout.splitlines()]
        occupied = []
        for item in networks:
            detail = json.loads(run(["docker", "network", "inspect", item["ID"]]).stdout)[0]
            occupied.extend(ipaddress.ip_network(entry["Subnet"]) for entry in (detail.get("IPAM",{}).get("Config") or []) if entry.get("Subnet"))
        subnet = next((ipaddress.ip_network(f"172.{index}.0.0/16") for index in range(16,32)
                       if not any(ipaddress.ip_network(f"172.{index}.0.0/16").overlaps(existing) for existing in occupied)), None)
        if subnet is None:
            raise RuntimeError("no unused private fixture subnet")
        run(["docker", "network", "create", "--internal", "--subnet", str(subnet), "--label", LABEL+"="+str(self.root), self.network])
        network = json.loads(run(["docker", "network", "inspect", self.network]).stdout)[0]
        subnet = ipaddress.ip_network(network["IPAM"]["Config"][0]["Subnet"])
        addresses = {node: str(subnet.network_address+index+2) for index,node in enumerate(["n1","n2","n3"])}
        for name in ["n1", "n2", "n3"]:
            self.certificate(name, server=True, addresses=addresses.values())
            self.certificate("route-"+name, server=True, addresses=addresses.values(), issuer="route-ca")
        for name in ["publisher", "consumer", "operator", "foreign-publisher", "unmapped"]:
            self.certificate(name)
        # Distinct accounts isolate the same subject string as well as role ACLs.
        publisher = {"user": "publisher", "permissions": {"publish": {"allow": ["celetixo.fixture.lifecycle"]}, "subscribe": {"allow": ["_INBOX.publisher.>"]}}}
        consumer = {"user": "consumer", "permissions": {"publish": {"allow": ["$JS.API.CONSUMER.INFO.LIFECYCLE.PROJECTION", "$JS.API.CONSUMER.MSG.NEXT.LIFECYCLE.PROJECTION", "$JS.ACK.LIFECYCLE.PROJECTION.>", "$JS.ACK.*.*.LIFECYCLE.PROJECTION.>"]}, "subscribe": {"allow": ["_INBOX.consumer.>"]}}}
        operator = {"user": "operator", "permissions": {"publish": {"allow": ["$JS.API.>", "celetixo.fixture.lifecycle", "$JS.ACK.>"]}, "subscribe": {"allow": ["_INBOX.operator.>"]}}}
        accounts = {"FIXTURE": {"jetstream": {"max_memory": 16*1024**2, "max_file": 64*1024**2, "max_streams": 2, "max_consumers": 4}, "users": [publisher, consumer, operator]},
                    "FOREIGN": {"users": [{"user": "foreign-publisher", "permissions": {"publish": {"allow": ["celetixo.fixture.lifecycle"]}, "subscribe": {"allow": ["_INBOX.foreign.>"]}}}]}}
        for node in ["n1", "n2", "n3"]:
            directory(self.root / node)
            directory(self.root / node / "data")
            directory(self.root / node / "certs")
            # A broker receives only its own keys and public trust anchors.
            for name in ["ca.pem","route-ca.pem",node+".pem",node+".key","route-"+node+".pem","route-"+node+".key"]:
                private(self.root / node / "certs" / name,(self.root / "certs" / name).read_bytes())
            config = {"server_name": node, "listen": "0.0.0.0:4222", "max_payload": 1048576,
                      "max_connections": 32, "max_pending": 2097152, "write_deadline": "2s",
                      "tls": {"cert_file": "/certs/"+node+".pem", "key_file": "/certs/"+node+".key", "ca_file": "/certs/ca.pem", "verify_and_map": True, "handshake_first": True, "timeout": 2},
                      "jetstream": {"store_dir": "/data", "max_memory_store": 16*1024**2, "max_file_store": 64*1024**2, "sync_interval": "always"},
                      "cluster": {"name": "celetixo-fixture", "listen": "0.0.0.0:6222", "no_advertise": True,
                                  "routes": ["nats-route://"+peer+":6222" for peer in ["n1","n2","n3"] if peer != node],
                                  "tls": {"cert_file": "/certs/route-"+node+".pem", "key_file": "/certs/route-"+node+".key", "ca_file": "/certs/route-ca.pem", "verify": True, "timeout": 2}},
                      "accounts": accounts}
            private(self.root / node / "nats.json", (json.dumps(config, indent=2)+"\n").encode())
            run(["docker", "run", "--rm", "--network=none", "--user", f"{os.getuid()}:{os.getgid()}",
                 "--cap-drop=ALL", "--security-opt=no-new-privileges", "--read-only", "--memory=128m", "--pids-limit=32",
                 "--mount", f"type=bind,src={self.root / node / 'certs'},dst=/certs,readonly",
                 "--mount", f"type=bind,src={self.root / node / 'nats.json'},dst=/nats.json,readonly", self.image, "-t", "-c", "/nats.json"])
        image_id = json.loads(run(["docker", "image", "inspect", self.image]).stdout)[0]["Id"]
        manifest = {"version": VERSION, "binary_sha256": BINARY_SHA256, "license_sha256": LICENSE_SHA256, "image_id": image_id,
                    "network": self.network, "nodes": ["n1","n2","n3"], "addresses": addresses, "sync_interval": "always", "data_retained_on_stop": True}
        private(self.root / "manifest.json", (json.dumps(manifest, indent=2)+"\n").encode())
        return manifest

    def start(self):
        manifest = self.prepare()
        result = run(["docker", "network", "inspect", self.network], check=False)
        if result.returncode:
            run(["docker", "network", "create", "--internal", "--label", LABEL+"="+str(self.root), self.network])
        else:
            network = json.loads(result.stdout)[0]
            if not network["Internal"] or network.get("Labels", {}).get(LABEL) != str(self.root):
                raise RuntimeError("unexpected network owner or egress policy")
        endpoints = []
        for node in manifest["nodes"]:
            item = self.inspect(node)
            if item is None:
                run(["docker", "create", "--name", self.name(node), "--label", LABEL+"="+str(self.root),
                     "--network", self.network, "--network-alias", node, "--ip", manifest["addresses"][node],
                     "--user", f"{os.getuid()}:{os.getgid()}", "--cap-drop=ALL", "--security-opt=no-new-privileges",
                     "--read-only", "--cpus=0.5", "--memory=256m", "--pids-limit=64", "--ulimit", "nofile=1024:1024",
                     "--log-driver=json-file", "--log-opt=max-size=2m", "--log-opt=max-file=2",
                     "--mount", f"type=bind,src={self.root / node / 'certs'},dst=/certs,readonly",
                     "--mount", f"type=bind,src={self.root / node / 'nats.json'},dst=/nats.json,readonly",
                     "--mount", f"type=bind,src={self.root / node / 'data'},dst=/data", self.image, "-c", "/nats.json"])
                item = self.inspect(node)
            if item["Image"] != manifest["image_id"]:
                raise RuntimeError("retained node has a different image")
            if not item["State"]["Running"]:
                run(["docker", "start", self.name(node)])
            item = self.inspect(node)
            if item["HostConfig"]["PortBindings"]:
                raise RuntimeError("internal fixture unexpectedly publishes host ports")
            endpoints.append("tls://"+manifest["addresses"][node]+":4222")
        configuration = {"servers": endpoints, "ca_file": str(self.root / "certs/ca.pem"),
                         "identities": {name: {"certificate": str(self.root / ("certs/"+name+".pem")), "private_key": str(self.root / ("certs/"+name+".key"))} for name in ["publisher","consumer","operator","foreign-publisher","unmapped"]}}
        path = self.root / "connection.json"
        if path.exists():
            if json.loads(path.read_text()) != configuration:
                raise RuntimeError("retained endpoint configuration changed; reconcile explicitly")
        else:
            private(path, (json.dumps(configuration, indent=2)+"\n").encode())
        context = ssl.create_default_context(cafile=configuration["ca_file"])
        context.load_cert_chain(configuration["identities"]["operator"]["certificate"], configuration["identities"]["operator"]["private_key"])
        deadline = time.monotonic()+30
        for endpoint in endpoints:
            host,port = endpoint.removeprefix("tls://").rsplit(":",1)
            while True:
                try:
                    with socket.create_connection((host,int(port)),timeout=2) as raw:
                        with context.wrap_socket(raw,server_hostname="localhost") as peer:
                            if not peer.recv(65536).startswith(b"INFO "):
                                raise RuntimeError("missing encrypted NATS INFO")
                            peer.sendall(b'CONNECT {"verbose":false,"pedantic":true}\r\nPING\r\n')
                            if b"PONG" not in peer.recv(65536):
                                raise RuntimeError("authenticated protocol readiness failed")
                    break
                except (OSError, RuntimeError):
                    if time.monotonic()>deadline:
                        raise RuntimeError("NATS TLS readiness deadline exceeded") from None
                    time.sleep(0.2)
        return {"running_nodes": 3, "connection_file": str(path), "tls_first": True, "mutual_tls": True,
                "route_tls": True, "internal_network": True, "host_ports_published": False, "host_to_internal_endpoints": endpoints, "sync_interval": "always",
                "replication_claim": "stream replica count and failover require integration verification"}

    def stop(self, selected=None):
        nodes = [selected] if selected else ["n1","n2","n3"]
        for node in nodes:
            item = self.inspect(node)
            if item and item["State"]["Running"]:
                run(["docker", "stop", "--time=15", self.name(node)], timeout=30)
        return {"stopped": nodes, "all_data_retained": True}

    def verify(self):
        manifest = json.loads((self.root / "manifest.json").read_text())
        network = json.loads(run(["docker","network","inspect",self.network]).stdout)[0]
        if not network["Internal"]:
            raise RuntimeError("fixture has network egress")
        for node in manifest["nodes"]:
            item = self.inspect(node)
            host = item["HostConfig"]
            if (not item["State"]["Running"] or item["Image"] != manifest["image_id"]
                    or not host["ReadonlyRootfs"] or host["Memory"] != 256*1024**2
                    or host["PidsLimit"] != 64 or host["NanoCpus"] != 500000000
                    or host["PortBindings"] or "ALL" not in host["CapDrop"]
                    or not any(value.startswith("no-new-privileges") for value in host["SecurityOpt"])):
                raise RuntimeError("node runtime differs from bounded fixture profile")
            mounted = next(mount for mount in item["Mounts"] if mount["Destination"] == "/certs")
            expected = self.root / node / "certs"
            if mounted["Source"] != str(expected) or mounted["RW"] or set(p.name for p in expected.iterdir()) != {
                    "ca.pem","route-ca.pem",node+".pem",node+".key","route-"+node+".pem","route-"+node+".key"}:
                raise RuntimeError("broker certificate mount exposes excess credentials")
        address = manifest["addresses"]["n1"]
        def context(ca="ca.pem", identity="operator"):
            value = ssl.create_default_context(cafile=str(self.root / "certs" / ca))
            if identity:
                value.load_cert_chain(str(self.root / "certs" / (identity+".pem")),str(self.root / "certs" / (identity+".key")))
            return value
        denied = {}
        for label,ctx,hostname in [("wrong_ca",context("unmapped.pem"),"localhost"),
                                   ("wrong_hostname",context(),"wrong.invalid"),
                                   ("missing_client_certificate",context(identity=None),"localhost")]:
            try:
                with socket.create_connection((address,4222),timeout=2) as peer:
                    with ctx.wrap_socket(peer,server_hostname=hostname) as secure:
                        secure.recv(65536)
            except ssl.SSLError:
                denied[label] = True
            else:
                raise RuntimeError("TLS security probe was unexpectedly admitted: "+label)
        # Routes have a separate trust root. A valid client credential must not
        # become a cluster-peer credential or inherit routing authority.
        with socket.create_connection((address,6222),timeout=2) as peer:
            with context("route-ca.pem","route-n1").wrap_socket(peer,server_hostname="localhost") as secure:
                if not secure.getpeercert():
                    raise RuntimeError("positive route TLS control failed")
        try:
            with socket.create_connection((address,6222),timeout=2) as peer:
                with context("route-ca.pem","publisher").wrap_socket(peer,server_hostname="localhost") as secure:
                    secure.sendall(b'CONNECT {}\r\nPING\r\n')
                    response = secure.recv(65536)
                    if not response:
                        denied["client_certificate_at_route"] = True
                    else:
                        raise RuntimeError("client certificate admitted to cluster route")
        except ssl.SSLError:
            denied["client_certificate_at_route"] = True
        return {"bounded_nodes":3,"separate_route_ca":True,"broker_mounts_only_own_keys":True,
                "denied_tls_probes":denied,"connection_file":str(self.root / "connection.json"),
                "stream_replication_and_recovery":"requires actual integration tests"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start","stop","status","verify","provision-tools"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--node", choices=["n1","n2","n3"])
    args = parser.parse_args()
    if os.getuid()==0 or platform.system()!="Linux" or platform.machine() not in ("x86_64","AMD64"):
        raise RuntimeError("the reviewed fixture requires non-root Linux x86_64")
    if not os.environ.get("DOCKER_HOST","unix://local").startswith("unix://"):
        raise RuntimeError("fixture requires a local Unix-socket Docker endpoint")
    endpoint=run(["docker","context","inspect","--format","{{.Endpoints.docker.Host}}"] ).stdout.strip()
    if not endpoint.startswith(b"unix://"):
        raise RuntimeError("fixture requires a local Unix-socket Docker context")
    fixture = Fixture(args.root,args.tools)
    if args.action == "provision-tools":
        result = provision_tools(args.tools)
    elif args.action == "start":
        result = fixture.start()
    elif args.action == "stop":
        result = fixture.stop(args.node)
    elif args.action == "verify":
        result = fixture.verify()
    else:
        result = {node: bool((fixture.inspect(node) or {}).get("State",{}).get("Running")) for node in ["n1","n2","n3"]}
    print(json.dumps(result,indent=2))


if __name__ == "__main__":
    main()

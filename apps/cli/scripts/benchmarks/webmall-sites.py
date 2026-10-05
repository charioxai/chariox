"""MP-08 / MP-10, WP-12: isolated deployment of pinned official WebMall shops.

Only placement, resource limits and URLs differ from upstream. Never execute
the upstream global-name restore/cleanup scripts on a shared builder.
"""
import argparse
import hashlib
import json
import os
import signal
import sys
import re
from pathlib import Path
import shutil
import socket
import subprocess
import time
import tempfile
import uuid
import urllib.request
import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent / "round2"))
from owned_processes import OwnedProcessTree
from webmall_readiness import probe_search

MP = ["MP-08", "MP-10", "MP-11"]
PREFIX = "r2next-webmall-r3"
PUBLIC_PREFIX = "benchwm-20261003"
parser = argparse.ArgumentParser()
parser.add_argument("operation", choices=["start", "cleanup", "probe"])
parser.add_argument("--upstream", required=True)
parser.add_argument("--lane", required=True)
parser.add_argument("--evidence", required=True)
parser.add_argument("--compose-binary", required=True)
args = parser.parse_args()
assert Path(args.compose_binary).is_absolute()
source, lane, evidence = map(Path, [args.upstream, args.lane, args.evidence])
root = lane / "sites"
root.mkdir(parents=True, exist_ok=True, mode=0o700)
evidence.mkdir(parents=True, exist_ok=True, mode=0o700)
ledger = root / "ownership.json"
owned = json.loads(ledger.read_text()) if ledger.exists() else {
    "mp_items": MP, "prefix": PREFIX, "containers": [], "volumes": [],
    "images": [], "network": PREFIX + "-webmall"}
assert owned["prefix"] == PREFIX
cleaning = args.operation == "cleanup"

def save():
    ledger.write_text(json.dumps(owned, indent=2) + "\n")

def guard():
    mem = int(next(s.split()[1] for s in Path("/proc/meminfo").read_text().splitlines()
                   if s.startswith("MemAvailable:"))) * 1024
    disk = shutil.disk_usage("/").free
    sample = {"mp_items": MP, "at": time.time(), "memAvailable": mem, "diskAvailable": disk}
    with (evidence / "resources.jsonl").open("a") as f:
        f.write(json.dumps(sample) + "\n")
    if mem < 16 * 1024**3 or disk < 10 * 1024**3:
        raise RuntimeError("MP-10 resource floor reached")

def command(argv, timeout=300, check=True, protected=False):
    start = time.time()
    with tempfile.TemporaryFile(dir=root) as stdout, tempfile.TemporaryFile(dir=root) as stderr:
        p = subprocess.Popen(argv, stdout=stdout, stderr=stderr, start_new_session=True)
        tree = OwnedProcessTree(p.pid)
        next_guard = 0
        while p.poll() is None:
            if not cleaning and time.time() >= next_guard:
                try:
                    guard()
                    next_guard = time.time() + 5
                except Exception:
                    tree.signal(signal.SIGTERM)
                    try: p.wait(timeout=5)
                    except subprocess.TimeoutExpired: tree.signal(signal.SIGKILL); p.wait()
                    raise
            if time.time() - start > timeout:
                tree.signal(signal.SIGKILL); p.wait()
                break
            time.sleep(0.1)
        stdout.seek(0); out = stdout.read()
        stderr.seek(0); err = stderr.read()
        # Only allowlisted diagnostics leave the private subprocess buffers.
        diagnostics = {name: needle in err or needle in out for name, needle in {
            "unsupportedJvmOption": b"Unrecognized VM option",
            "searchNotResponding": b"Elasticsearch nodes are not responding",
            "alreadySyncing": b"currently syncing",
            "unsupportedSearchVersion": b"Elasticsearch version",
            "phpFatal": b"Fatal error",
            "missingCommand": b"not a registered wp command",
            "connectionRefused": b"Connection refused",
        }.items()}
    with (evidence / "site-commands.jsonl").open("a") as f:
        f.write(json.dumps({"mp_items": MP, "argv": argv, "exitCode": p.returncode,
                            "seconds": time.time() - start, "diagnosticFlags": diagnostics}) + "\n")
    if check and p.returncode:
        # Runtime logs may contain synthetic DB credentials. Do not export them.
        raise RuntimeError(f"MP-10 command failed: {argv[:3]}, exit {p.returncode}")
    return out.decode(), p.returncode

def docker(*argv, **kwargs):
    if argv[0] == "run":
        name = PREFIX + "-restore-" + uuid.uuid4().hex[:8]
        owned["containers"].append(name); save()
        argv = ("run", "--name", name, *argv[1:])
    return command(["docker", *argv], **kwargs)

def inspect(kind, name):
    out, code = docker(kind, "inspect", name, check=False)
    return json.loads(out)[0] if code == 0 else None

def cleanup():
    global cleaning
    cleaning = True
    for name in reversed(owned["containers"]):
        obj = inspect("container", name)
        if obj:
            assert obj["Config"]["Labels"].get("io.chariox.benchmark.lane") == PREFIX
            docker("rm", "-f", name)
    for name in owned["volumes"]:
        obj = inspect("volume", name)
        if obj:
            assert obj["Labels"].get("io.chariox.benchmark.lane") == PREFIX
            docker("volume", "rm", name)
    obj = inspect("network", owned["network"])
    if obj:
        assert obj["Labels"].get("io.chariox.benchmark.lane") == PREFIX
        docker("network", "rm", owned["network"])
    # Never remove shared images, even if pulled by this lane.
    result = {"mp_items": MP, "at": time.time(), "containersGone": all(
        inspect("container", n) is None for n in owned["containers"]),
        "volumesGone": all(inspect("volume", n) is None for n in owned["volumes"]),
        "networkGone": inspect("network", owned["network"]) is None,
        "sharedImageCachePreserved": True}
    (evidence / "sites-cleanup.json").write_text(json.dumps(result, indent=2) + "\n")

def parse_search_metadata(output):
    lines = [line.removeprefix("MP10_PUBLIC_SEARCH=") for line in output.splitlines() if line.startswith("MP10_PUBLIC_SEARCH=")]
    assert len(lines) == 1, "MP-10 search metadata unavailable"
    result = json.loads(lines[0])
    assert set(result) == {"host", "index", "publishedProducts"}, "MP-10 unexpected search metadata"
    assert isinstance(result["host"], str) and result["host"].rstrip("/") == "http://elasticsearch:9200", "MP-10 WordPress search wiring mismatch"
    return result

def collect_search_readiness():
    initial = json.loads((evidence / "catalog-restored.json").read_text())
    shops = []
    for i in range(1, 5):
        name = PREFIX + f"-wordpress_shop{i}"
        # The plugin's constant/filter can override the stored option. Probe the
        # exact effective endpoint used by sync/search, with explicit framing
        # so incidental PHP notices cannot contaminate the public values.
        output, _ = docker("exec", name, "wp", "eval", r'echo "\nMP10_PUBLIC_SEARCH=" . json_encode(["host" => \ElasticPress\Utils\get_host(), "index" => \ElasticPress\Indexables::factory()->get("post")->get_index_name(), "publishedProducts" => (int) wp_count_posts("product")->publish]) . "\n";')
        metadata = parse_search_metadata(output)
        shops.append({"shop": i, "index": metadata["index"], "publishedProducts": metadata["publishedProducts"],
                      "expectedProducts": initial[i-1]["publishedProducts"]})
    ports = json.loads((root / "ports.json").read_text())
    return probe_search("http://127.0.0.1:" + str(ports["elasticsearch"]), shops)

if args.operation == "probe":
    result = collect_search_readiness()
    target = evidence / "search-admission.json"
    target.write_text(json.dumps(result, indent=2) + "\n")
    print("MP-08/MP-10 search admission ready")
    raise SystemExit(0)

if args.operation == "cleanup":
    cleanup()
    raise SystemExit(0)

stage = "provenance"
try:
    assert command(["git", "-C", str(source), "rev-parse", "HEAD"])[0].strip() == "2697d35cdfcfedcf1ade89b7ad86722db8daa8a7"
    compose = yaml.safe_load((source / "docker_all/docker-compose.yml").read_text())
    # Official synthetic fixture environment; never print its contents.
    variables = {}
    for line in (source / ".env.example").read_text().splitlines():
        key, sep, value = line.partition("=")
        if sep and not key.startswith("#"):
            variables[key.strip()] = value.strip().strip("\"'")
    ports = {}
    for name in ["frontend", "elasticsearch", *[f"shop{i}" for i in range(1, 5)]]:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0)); ports[name] = s.getsockname()[1]
    (root / "ports.json").write_text(json.dumps(ports) + "\n")
    variables.update({f"SHOP{i}_PORT": str(ports[f"shop{i}"]) for i in range(1, 5)})
    variables["FRONTEND_PORT"] = str(ports["frontend"])
    for key, value in variables.items():
        compose = yaml.safe_load(yaml.safe_dump(compose).replace("${" + key + "}", value))
    pins = {}
    stage = "official_image_pull"
    frozen_images = json.loads((lane / "site-image-pins.json").read_text())
    required = {s["image"] for s in compose["services"].values()} | {"busybox:latest"}
    assert required == {p["tag"] for p in frozen_images}, "MP-10 incomplete frozen image pins"
    for pin in frozen_images:
        ref = pin["repoDigests"][0]
        assert "@sha256:" in ref
        docker("pull", ref, timeout=600)
        obj = inspect("image", ref)
        assert obj["Id"] == pin["id"], "MP-10 frozen image mismatch"
        pins[pin["tag"]] = obj["Id"]
        owned["images"].append(pin); save()
    (evidence / "site-image-pins.json").write_text(json.dumps(owned["images"], indent=2) + "\n")
    stage = "official_backup_download"
    backups = root / "backup"; backups.mkdir(exist_ok=True)
    hashes = []
    for kind in ["mariadb", "wordpress"]:
        for i in range(1, 5):
            name = f"{kind}_data_shop{i}.tar.gz"
            target = backups / name
            url = "https://data.dws.informatik.uni-mannheim.de/webmall/backup/" + name
            digest = hashlib.sha256()
            if target.exists():
                # Reuse a complete official backup only after checking the frozen receipt.
                pins_file = lane / "backup-pins.json"
                old = json.loads(pins_file.read_text()) if pins_file.exists() else []
                pin = next((p for p in old if p["file"] == name), None)
                with target.open("rb") as f:
                    while chunk := f.read(1024**2):
                        guard(); digest.update(chunk)
                assert pin and pin["bytes"] == target.stat().st_size and pin["sha256"] == digest.hexdigest()
            else:
                with urllib.request.urlopen(url, timeout=60) as response, target.open("wb") as f:
                    while chunk := response.read(1024**2):
                        guard(); digest.update(chunk); f.write(chunk)
            pin = next(p for p in json.loads((lane / "backup-pins.json").read_text()) if p["file"] == name)
            assert target.stat().st_size == pin["bytes"] and digest.hexdigest() == pin["sha256"], "MP-10 frozen backup mismatch"
            hashes.append({"file": name, "bytes": target.stat().st_size, "sha256": digest.hexdigest(), "url": url})
            (evidence / "backup-pins.json").write_text(json.dumps(hashes, indent=2) + "\n")
    # Frozen inventory is never replaced by freshly downloaded identities.
    stage = "isolated_volume_restore"
    for name in compose["volumes"]:
        exact = PREFIX + "-" + name
        assert inspect("volume", exact) is None, "MP-10 refuse existing fixture volume"
        owned["volumes"].append(exact); save()
        docker("volume", "create", "--label", "io.chariox.benchmark.lane=" + PREFIX, exact)
        compose["volumes"][name] = {"external": True, "name": exact}
        if name.startswith("woocommerce_"):
            backup = name.replace("woocommerce_", "") + ".tar.gz"
            docker("run", "--rm", "--label", "io.chariox.benchmark.lane=" + PREFIX,
                   "--memory", "256m", "--cpus", "1", "-v", exact + ":/volume",
                   "-v", str(backups) + ":/backup:ro", pins["busybox:latest"],
                   "tar", "xzf", "/backup/" + backup, "-C", "/volume", timeout=300)
    urls = {f"SHOP{i}_URL": f"http://{PUBLIC_PREFIX}-shop{i}.local:8080" for i in range(1, 5)}
    urls["FRONTEND_URL"] = f"http://{PUBLIC_PREFIX}-frontend.local"
    for i in range(1, 5):
        text = (source / f"docker_all/deployed_wp_config_local/shop_{i}.php").read_text()
        text = text.replace(f"http://localhost:SHOP{i}_PORT_PLACEHOLDER", urls[f"SHOP{i}_URL"])
        config = root / f"shop_{i}.php"; config.write_text(text); config.chmod(0o600)
        docker("run", "--rm", "--label", "io.chariox.benchmark.lane=" + PREFIX,
               "-v", PREFIX + f"-woocommerce_wordpress_data_shop{i}:/volume",
               "-v", str(config) + ":/source:ro", pins["busybox:latest"], "sh", "-c",
               "cp /source /volume/wp-config.php && chown 1001:0 /volume/wp-config.php && chmod 600 /volume/wp-config.php")
    shutil.copy(source / "docker_all/index.html", root / "index.html")
    for name, service in compose["services"].items():
        exact = PREFIX + "-" + name
        owned["containers"].append(exact); save()
        assert inspect("container", exact) is None
        service["container_name"] = exact
        service["image"] = pins[service["image"]]
        service["labels"] = {"io.chariox.benchmark.lane": PREFIX}
        service["cpus"] = 0.5
        service["mem_limit"] = "2048m" if name == "elasticsearch" else "512m" if name.startswith("wordpress") else "384m" if name.startswith("mariadb") else "128m"
        service["pids_limit"] = 256
        if name == "elasticsearch":
            # Upstream's ARM-only JVM flag is invalid on the reserved x86 builder.
            env = service.get("environment", [])
            if isinstance(env, list):
                service["environment"] = [v.replace(" -XX:UseSVE=0", "") for v in env]
            else:
                env["ES_JAVA_OPTS"] = env["ES_JAVA_OPTS"].replace(" -XX:UseSVE=0", "")
            # Pinned image's CLI launcher also inherits the ARM-only flag.
            if isinstance(service["environment"], list):
                service["environment"].append("CLI_JAVA_OPTS=")
            else:
                service["environment"]["CLI_JAVA_OPTS"] = ""
        alias = f"{PUBLIC_PREFIX}-shop{name[-1]}.local" if name.startswith("wordpress") else f"{PUBLIC_PREFIX}-frontend.local" if name == "webmall_frontend" else name
        service["networks"] = {"webmall": {"aliases": [alias]}}
        if "ports" in service:
            internal = 8080 if name.startswith("wordpress") else 80 if name == "webmall_frontend" else 9200
            portkey = f"shop{name[-1]}" if name.startswith("wordpress") else "frontend" if name == "webmall_frontend" else "elasticsearch"
            service["ports"] = [f"127.0.0.1:{ports[portkey]}:{internal}"]
        service["volumes"] = [v.replace("./", str(source / "docker_all") + "/") for v in service.get("volumes", [])]
        if name == "webmall_frontend": service["volumes"] = [str(root / "index.html") + ":/usr/share/nginx/html/index.html:ro"]
    compose["networks"] = {"webmall": {"name": owned["network"], "labels": {"io.chariox.benchmark.lane": PREFIX}}}
    (root / "compose.yaml").write_text(yaml.safe_dump(compose)); (root / "compose.yaml").chmod(0o600)
    stage = "official_services_start"
    command([args.compose_binary, "-p", PREFIX, "-f", str(root / "compose.yaml"), "up", "-d"], timeout=300)
    stage = "wordpress_readiness"
    catalog = []
    for i in range(1, 5):
        name = PREFIX + f"-wordpress_shop{i}"
        for attempt in range(60):
            if docker("exec", name, "wp", "core", "is-installed", "--path=/opt/bitnami/wordpress", check=False)[1] == 0: break
            time.sleep(2)
        else: raise RuntimeError("MP-10 official WordPress did not become ready")
        docker("exec", name, "/bin/bash", "/usr/local/bin/fix_urls_deploy.sh",
               f"https://webmall-{i}.informatik.uni-mannheim.de/", urls[f"SHOP{i}_URL"] + "/", timeout=180)
        count = docker("exec", name, "wp", "post", "list", "--post_type=product", "--post_status=publish", "--format=count", "--skip-plugins", "--skip-themes", "--path=/opt/bitnami/wordpress")[0].strip()
        parsed = re.match(r"^(\d+)", count)
        assert parsed, "MP-10 catalog count unavailable"
        catalog.append({"shop": i, "publishedProducts": int(parsed[1])})
    assert all(s["publishedProducts"] > 1000 for s in catalog), "MP-10 fixture catalog not populated"
    (lane / "site-urls.json").write_text(json.dumps(urls, indent=2) + "\n")
    (evidence / "catalog-restored.json").write_text(json.dumps(catalog) + "\n")
    public = {"mp_items": MP, "status": "ready", "urls": urls, "catalog": catalog,
              "documentedCatalog": [1150, 1095, 1156, 1020],
              "catalogMatchesWebsite": [s["publishedProducts"] for s in catalog] == [1150, 1095, 1156, 1020],
              "loopbackPorts": ports, "network": owned["network"], "source": "2697d35cdfcfedcf1ade89b7ad86722db8daa8a7"}
    stage = "search_service_readiness"
    for attempt in range(90):
        guard()
        try:
            with urllib.request.urlopen("http://127.0.0.1:" + str(ports["elasticsearch"]) + "/_cluster/health", timeout=4) as response:
                health = json.load(response)
            if health.get("status") in ("yellow", "green") and health.get("timed_out") is False:
                break
        except (OSError, ValueError): pass
        time.sleep(2)
    else: raise RuntimeError("MP-10 search service readiness deadline")
    stage = "search_index_sync"
    for i in range(1, 5):
        docker("exec", PREFIX + f"-wordpress_shop{i}", "wp", "elasticpress", "sync", "--setup", "--yes", "--stop-on-error", timeout=300)
    stage = "search_admission"
    public["search"] = collect_search_readiness()
    (evidence / "sites-readiness.json").write_text(json.dumps(public, indent=2) + "\n")
    print("MP-08 / MP-10 isolated WebMall shops ready")
except Exception as error:
    failure = str(error) if isinstance(error, (RuntimeError, AssertionError)) else type(error).__name__
    (evidence / "sites-readiness.json").write_text(json.dumps({"mp_items": MP, "status": "RED", "firstFailingSeam": stage, "failure": failure}, indent=2) + "\n")
    print("MP-08 / MP-10 RED: " + stage + ": " + failure)
    cleanup()
    raise SystemExit(1)

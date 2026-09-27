"""Read-only, bounded host census. No shell, docker exec, credentials or file contents."""
import json
import os
import re
import selectors
import stat
import subprocess
import sys
import time

LIMIT = 4096
ENGINE_ENDPOINT = None


def bounded(values, maximum=LIMIT):
    result = []
    for value in values:
        result.append(value)
        if len(result) > maximum:
            raise ValueError("census cardinality limit")
    return result


def docker(*args):
    if ENGINE_ENDPOINT is None:
        raise ValueError("explicit Docker engine required")
    environment = {key: value for key, value in os.environ.items() if not key.startswith("DOCKER_")}
    child = subprocess.Popen(["docker", "--host", ENGINE_ENDPOINT, *args],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
    output, size = [], 0
    deadline = time.monotonic() + 3
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            selector.register(child.stderr, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError("Docker deadline")
                for key, _ in selector.select(remaining):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    size += len(chunk)
                    if size > 1024 * 1024:
                        raise ValueError("Docker output limit")
                    if key.fileobj is child.stdout:
                        output.append(chunk)
        if child.wait(timeout=max(0.01, deadline - time.monotonic())) != 0:
            raise ValueError("Docker inspection failed")
        return b"".join(output).decode()
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=1)
        child.stdout.close()
        child.stderr.close()


def process_identity(pid):
    prefix = "/proc/" + str(pid)
    raw = open(prefix + "/stat").read()
    fields = raw[raw.rindex(")") + 2:].split()
    with open(prefix + "/cgroup") as stream:
        cgroup = stream.read(65537)
    if len(cgroup) > 65536:
        raise ValueError("process cgroup limit")
    return {"pid": int(pid), "startTicks": fields[19],
            "pidNamespace": os.readlink(prefix + "/ns/pid"),
            "netNamespace": os.readlink(prefix + "/ns/net"),
            "rssBytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"), "cgroup": cgroup}


def belongs_to_container(cgroup, container_ids):
    return any(re.search(r"(?<![a-f0-9])" + re.escape(value) + r"(?![a-f0-9])", cgroup)
               for value in container_ids)


def same_process(before, after):
    return all(before[key] == after[key] for key in ("pid", "startTicks", "pidNamespace", "netNamespace"))


def same_process_lifetime(before, after):
    return before["pid"] == after["pid"] and before["startTicks"] == after["startTicks"]


def path_identity(path):
    value = os.lstat(path)
    return {"path": path, "device": str(value.st_dev), "inode": str(value.st_ino),
            "kind": "directory" if stat.S_ISDIR(value.st_mode) else "file" if stat.S_ISREG(value.st_mode) else "other"}


def mount_identity(path):
    candidates = []
    with open("/proc/self/mountinfo") as stream:
        for line in bounded(stream, 4096):
            fields = line.split()
            point = re.sub(r"\\([0-7]{3})", lambda match: chr(int(match[1], 8)), fields[4])
            if path == point or path.startswith(point.rstrip("/") + "/"):
                candidates.append((len(point), {"mountId": fields[0], "parentMountId": fields[1],
                                               "mountDevice": fields[2], "mountPoint": point}))
    if not candidates:
        raise ValueError("path mount unavailable")
    return max(candidates, key=lambda item: item[0])[1]


def main():
    global ENGINE_ENDPOINT
    if os.geteuid() != 0:
        sys.stderr.write("physical census requires a root SSH identity\n")
        raise SystemExit(77)
    payload = sys.stdin.buffer.read(256 * 1024 + 1)
    if len(payload) > 256 * 1024:
        raise ValueError("input limit")
    request = json.loads(payload)
    engine = request["engine"]
    if not re.fullmatch(r"unix://[/a-zA-Z0-9_.-]+\.sock", engine["endpoint"]):
        raise ValueError("explicit Unix Docker endpoint required")
    ENGINE_ENDPOINT = engine["endpoint"]
    if docker("info", "--format", "{{.ID}}").strip() != engine["id"]:
        raise ValueError("Docker engine identity mismatch")
    resources = bounded(request.get("resources", []), 32)
    retained = request.get("retained") or {}
    roots = bounded(request.get("paths", []), 32)
    boot = open("/proc/sys/kernel/random/boot_id").read().strip()
    containers, volumes = {}, {}
    for owner in resources:
        labels = {"io.chariox.slice.id": owner["sliceId"],
                  "io.chariox.slice.owner-kernel-id": owner["ownerKernelId"],
                  "io.chariox.slice.owner-machine-id": owner["ownerMachineId"]}
        filters = []
        for key, value in labels.items():
            if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9_.:-]{1,160}", value):
                raise ValueError("invalid ownership identity")
            filters.extend(["--filter", "label=" + key + "=" + value])
        ids = bounded(docker("ps", "--all", "--no-trunc", "--quiet", *filters).split(), 64)
        if any(not re.fullmatch(r"[0-9a-f]{64}", item) for item in ids):
            raise ValueError("invalid immutable container id")
        if ids:
            for item in json.loads(docker("container", "inspect", *ids)):
                if any(item["Config"].get("Labels", {}).get(key) != value for key, value in labels.items()):
                    raise ValueError("container ownership changed")
                containers[item["Id"]] = {"id": item["Id"], "labels": labels,
                    "initPid": item["State"]["Pid"], "running": item["State"]["Running"],
                    "mounts": [{key: mount.get(key) for key in ("Type", "Name", "Source", "Destination")}
                               for mount in item["Mounts"]]}
        names = bounded(docker("volume", "ls", "--quiet", *filters).split(), 64)
        if names:
            for item in json.loads(docker("volume", "inspect", *names)):
                if any(item.get("Labels", {}).get(key) != value for key, value in labels.items()):
                    raise ValueError("volume ownership changed")
                identity = path_identity(item["Mountpoint"])
                volumes[item["Name"]] = {"name": item["Name"], "labels": labels, **identity}
    # An ID remembered from a previous generation is checked independently of
    # current labels. Label removal must not make retained resources disappear.
    all_ids = set(bounded(docker("ps", "--all", "--no-trunc", "--quiet").split()))
    retained_container_ids = bounded(retained.get("containerIds", []), 256)
    retained_containers = [value for value in retained_container_ids if value in all_ids]
    all_volume_names = set(bounded(docker("volume", "ls", "--quiet").split()))
    retained_volumes = [value for value in bounded(retained.get("volumeNames", []), 256) if value in all_volume_names]
    known_container_ids = set(containers) | set(retained_container_ids)
    for item in containers.values():
        if item["running"]:
            item["initIdentity"] = process_identity(item["initPid"])
            if not belongs_to_container(item["initIdentity"]["cgroup"], {item["id"]}):
                raise ValueError("container init cgroup identity unavailable")
            if item["initIdentity"]["pidNamespace"] == os.readlink("/proc/1/ns/pid"):
                raise ValueError("host PID namespace is not an owned container namespace")
    processes = []
    total_rss = 0
    for entry in bounded(os.listdir("/proc"), 32768):
        if not entry.isdigit():
            continue
        try:
            identity = process_identity(entry)
            total_rss += identity["rssBytes"]
            if belongs_to_container(identity["cgroup"], known_container_ids):
                processes.append(identity)
        except (FileNotFoundError, ProcessLookupError):
            continue
    bounded(processes)
    profiles = []
    for identity in processes:
        prefix = "/proc/" + str(identity["pid"])
        try:
            with open(prefix + "/cmdline", "rb") as stream:
                raw = stream.read(65537)
            if len(raw) > 65536:
                raise ValueError("owned process argument bound")
            args = raw.decode().split("\0")
            if os.path.basename(args[0]).lower() not in ("chromium", "chromium-browser", "chrome", "google-chrome", "google-chrome-stable"):
                continue
            if any(arg.startswith("--type=") for arg in args):
                continue
            profile = next((arg.split("=", 1)[1] for arg in args if arg.startswith("--user-data-dir=")), None)
            if profile is None and "--user-data-dir" in args:
                profile = args[args.index("--user-data-dir") + 1]
            if not profile or not profile.startswith("/") or os.path.normpath(profile) != profile:
                raise ValueError("actual owned browser profile path unavailable")
            container = next((item for item in containers.values()
                              if belongs_to_container(identity["cgroup"], {item["id"]})), None)
            if container is None:
                raise ValueError("owned browser profile container coverage unavailable")
            mounts = [mount for mount in container["mounts"] if mount["Type"] == "volume"
                      and mount["Name"] in volumes and (profile == mount["Destination"]
                      or profile.startswith(mount["Destination"].rstrip("/") + "/"))]
            if not mounts:
                raise ValueError("owned browser profile is outside observed owned volumes")
            mount = max(mounts, key=lambda item: len(item["Destination"]))
            host_path = mount["Source"].rstrip("/") + profile[len(mount["Destination"]):]
            if os.path.realpath(host_path) != host_path:
                raise ValueError("profile symlink coverage is unavailable")
            observed = path_identity(host_path)
            actual = os.stat(prefix + "/root" + profile)
            if observed["kind"] != "directory" or (observed["device"], observed["inode"]) != (str(actual.st_dev), str(actual.st_ino)):
                raise ValueError("browser profile mount/inode mismatch")
            if not same_process(identity, process_identity(identity["pid"])):
                raise ValueError("browser changed during profile observation")
            profiles.append({**observed, "process": identity, "containerId": container["id"], "volumeName": mount["Name"]})
        except (FileNotFoundError, ProcessLookupError):
            continue
    residual_processes = []
    for old in bounded(retained.get("processes", [])):
        try:
            actual = process_identity(old["pid"])
            # Namespace/cgroup escape is residue, not disappearance. Start ticks
            # still distinguish this process from a reused PID on the same boot.
            if same_process_lifetime(old, actual):
                residual_processes.append(actual)
        except (FileNotFoundError, ProcessLookupError):
            pass
    listeners = []
    listening_processes = {str(item["pid"]) + ":" + item["startTicks"]: item for item in processes + residual_processes}
    for identity in listening_processes.values():
        prefix = "/proc/" + str(identity["pid"])
        try:
            sockets = set()
            for fd in bounded(os.listdir(prefix + "/fd")):
                try:
                    target = os.readlink(prefix + "/fd/" + fd)
                    if re.fullmatch(r"socket:\[[0-9]+\]", target):
                        sockets.add(target[8:-1])
                except FileNotFoundError:
                    continue
            for protocol in ("tcp", "tcp6", "udp", "udp6"):
                with open(prefix + "/net/" + protocol) as stream:
                    next(stream)
                    rows = bounded(stream)
                for row in rows:
                    fields = row.split()
                    if fields[9] in sockets and (protocol.startswith("udp") or fields[3] == "0A"):
                        listeners.append({"process": identity, "protocol": protocol,
                                          "socketInode": fields[9], "localAddress": fields[1]})
            if not same_process(identity, process_identity(identity["pid"])):
                raise ValueError("process changed during listener inspection")
        except (FileNotFoundError, ProcessLookupError):
            continue
    paths, mounts = [], []
    for root in roots:
        if not isinstance(root, str) or not root.startswith("/") or root == "/" or os.path.normpath(root) != root:
            raise ValueError("explicit narrow absolute observer path required")
        if os.path.realpath(root) != root:
            raise ValueError("observer root symlink refused")
        identity = path_identity(root)
        if identity["kind"] != "directory":
            raise ValueError("observer root must be a directory")
        usage = os.statvfs(root)
        mount = mount_identity(root)
        mounts.append({**identity, **mount, "totalBytes": usage.f_blocks * usage.f_frsize,
                       "freeBytes": usage.f_bfree * usage.f_frsize})
        for current, directories, files in os.walk(root, followlinks=False):
            if os.path.relpath(current, root).count(os.sep) > 16:
                raise ValueError("observer path depth limit")
            for name in directories + files:
                paths.append(path_identity(os.path.join(current, name)))
                bounded(paths)
        if path_identity(root) != identity or mount_identity(root) != mount:
            raise ValueError("observer path or mount changed during census")
    retained_paths = []
    for old in bounded(retained.get("paths", [])):
        try:
            actual = path_identity(old["path"])
            if (actual["device"], actual["inode"]) == (old["device"], old["inode"]):
                retained_paths.append(actual)
        except FileNotFoundError:
            pass
    retained_profiles = []
    for old in bounded(retained.get("profiles", []), 256):
        try:
            actual = path_identity(old["path"])
            if (actual["device"], actual["inode"]) == (old["device"], old["inode"]):
                retained_profiles.append(actual)
        except FileNotFoundError:
            pass
    if boot != open("/proc/sys/kernel/random/boot_id").read().strip():
        raise ValueError("host rebooted during observation")
    listeners = list({(item["process"]["netNamespace"], item["protocol"], item["socketInode"]): item
                      for item in listeners}.values())
    print(json.dumps({"schema": "chariox.managed_parity.host_observation.v1", "bootId": boot,
          "mountNamespace": os.readlink("/proc/self/ns/mnt"), "engineId": engine["id"],
          "hostContainerIds": sorted(all_ids), "hostVolumeNames": sorted(all_volume_names),
          "containers": list(containers.values()), "volumes": list(volumes.values()),
          "profiles": profiles, "retainedProfiles": retained_profiles,
          "processes": processes, "listeners": listeners, "paths": paths, "mounts": mounts,
          "retainedContainers": retained_containers, "retainedVolumes": retained_volumes,
          "retainedProcesses": residual_processes, "retainedPaths": retained_paths,
          "hostRssBytes": total_rss}, separators=(",", ":")))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        sys.stderr.write("bounded physical census unavailable\n")
        sys.exit(1)

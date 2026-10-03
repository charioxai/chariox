#!/usr/bin/env python3
"""Host Docker authority: retire only browser records captured after container exit."""
import io
import base64
from datetime import datetime
import json
import os
import re
import selectors
import subprocess
import sys
import tarfile
import time

ROOT = "/tmp/chariox-browser-lifecycle-1001"
UPLOAD_ROOT = "/tmp/chariox-browser-uploads-1001"
LIMIT = 2 * 1024 * 1024


def timestamp(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?Z", value):
        raise RuntimeError("Docker generation timestamp rejected")
    try:
        seconds, _, fraction = value[:-1].partition(".")
        return datetime.fromisoformat(seconds), int(fraction.ljust(9, "0"))
    except ValueError as error:
        raise RuntimeError("Docker generation timestamp rejected") from error


def command(*args, data=None):
    child = subprocess.Popen(["docker", *args], stdin=subprocess.PIPE if data else subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    output = bytearray()
    errors = bytearray()
    deadline = time.monotonic() + 10
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            selector.register(child.stderr, selectors.EVENT_READ)
            offset = 0
            if data:
                os.set_blocking(child.stdin.fileno(), False)
                selector.register(child.stdin, selectors.EVENT_WRITE)
            size = 0
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError("Docker reconciliation deadline")
                for key, _ in selector.select(remaining):
                    if key.fileobj is child.stdin:
                        offset += os.write(child.stdin.fileno(), data[offset:offset + 65536])
                        if offset == len(data):
                            selector.unregister(child.stdin)
                            child.stdin.close()
                        continue
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    size += len(chunk)
                    if size > LIMIT:
                        raise RuntimeError("Docker reconciliation output bound")
                    if key.fileobj is child.stdout:
                        output.extend(chunk)
                    else:
                        errors.extend(chunk)
        if child.wait(timeout=max(.01, deadline - time.monotonic())):
            raise RuntimeError("Docker reconciliation command failed")
        return bytes(output)
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=1)
        for stream in (child.stdin, child.stdout, child.stderr):
            if stream and not stream.closed:
                stream.close()


def snapshot(docker, target, labels):
    engine = docker("info", "--format", "{{.ID}}").decode().strip()
    rows = json.loads(docker("inspect", "--type=container", target))
    if not engine or len(rows) != 1 or not re.fullmatch("[a-f0-9]{64}", rows[0]["Id"]):
        raise RuntimeError("immutable Docker identity required")
    item = rows[0]
    if item.get("HostConfig", {}).get("PidMode") not in ("", "private"):
        raise RuntimeError("private container PID namespace required")
    if not labels or any(not value or item.get("Config", {}).get("Labels", {}).get(key) != value for key, value in labels.items()):
        raise RuntimeError("kernel-owned slice labels required")
    state = item["State"]
    if state["Status"] == "created" and not state["Running"] and state["Pid"] == 0:
        return None
    if state["Status"] not in ("exited", "dead") or state["Running"] or state["Paused"] or state["Restarting"] or state["Pid"] != 0:
        raise RuntimeError("exited container without live PID required")
    if timestamp(state["StartedAt"]) > timestamp(state["FinishedAt"]):
        raise RuntimeError("Docker generation timestamp order rejected")
    return {"engine": engine, "id": item["Id"], "started": state["StartedAt"], "finished": state["FinishedAt"]}


def receipts(archive, proof):
    files = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:*") as source:
        for index, member in enumerate(source):
            name = member.name
            if index >= 512 or name.startswith("/") or ".." in name.split("/"):
                raise RuntimeError("lifecycle archive path/cardinality rejected")
            if member.isdir() and name.rstrip("/") in (".", "chariox-browser-lifecycle-1001"):
                if member.uid != 1001 or member.mode & 0o777 != 0o700:
                    raise RuntimeError("lifecycle archive root ownership rejected")
                continue
            parts = name.split("/")
            if len(parts) == 2 and parts[0] in (".", "chariox-browser-lifecycle-1001"):
                name = parts[1]
            if re.fullmatch(r"record-[a-z0-9_]{8}", name):
                if not member.isfile() or member.uid != 1001 or member.mode & 0o777 != 0o600 or member.size > 4096:
                    raise RuntimeError("invalid interrupted lifecycle write")
                # An interrupted atomic write has no authority. Do not parse,
                # promote or copy it into retirement evidence.
                continue
            if not re.fullmatch(r"(?:[a-f0-9]{32}(?:\.retired|\.container-retired)?\.json|[a-f0-9]{64}\.json|launch\.lock)", name):
                raise RuntimeError("unexpected lifecycle archive path")
            if not member.isfile() or member.uid != 1001 or member.mode & 0o777 != 0o600 or member.size > 4096 or name in files:
                raise RuntimeError("invalid lifecycle archive entry")
            files[name] = source.extractfile(member).read()
    records = {}
    retired_count = 0
    for name, raw in files.items():
        if not re.fullmatch(r"[a-f0-9]{32}\.json", name):
            continue
        record = json.loads(raw)
        if set(record) != {"version", "instance", "profile", "supervisor", "browser"} or record["version"] != 1 or record["instance"] + ".json" != name:
            raise RuntimeError("invalid lifecycle owner schema")
        if not isinstance(record["profile"], str) or not record["profile"].startswith("/") or len(record["profile"]) > 2048:
            raise RuntimeError("invalid profile binding")
        for role in ("supervisor", "browser"):
            process = record[role]
            if role == "browser" and process is None:
                continue
            if not isinstance(process, dict) or set(process) != {"pid", "start", "group", "session", "uid", "boot", "namespace"}:
                raise RuntimeError("invalid process identity")
            if process["uid"] != 1001 or any(type(process[key]) is not int or process[key] <= 0 for key in ("pid", "group", "session")) or not re.fullmatch(r"[0-9]+", process["start"]) or not re.fullmatch(r"[a-f0-9-]{36}", process["boot"]) or not re.fullmatch(r"pid:\[[0-9]+\]", process["namespace"]):
                raise RuntimeError("invalid process identity values")
        retired = record["instance"] + ".retired.json"
        if retired in files:
            if json.loads(files[retired]) != record:
                raise RuntimeError("different existing retirement receipt")
        else:
            records[retired] = raw
            retired_count += 1
        marker = record["instance"] + ".container-retired.json"
        container_proof = {"version": 1, "authority": "docker-stopped-container", "engineId": proof["engine"],
                           "containerId": proof["id"], "startedAt": proof["started"], "finishedAt": proof["finished"], "lifetime": record}
        if marker in files:
            existing = json.loads(files[marker])
            same = set(existing) == set(container_proof) and all(existing[key] == container_proof[key]
                for key in ("version", "authority", "engineId", "containerId", "lifetime"))
            try:
                started = timestamp(existing["startedAt"])
                finished = timestamp(existing["finishedAt"])
                current_finished = timestamp(proof["finished"])
                same = same and started <= finished <= current_finished
            except (ValueError, TypeError, KeyError, AttributeError, RuntimeError):
                same = False
            if not same:
                raise RuntimeError("different existing container retirement proof")
        else:
            records[marker] = json.dumps(container_proof).encode()
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as target:
        for name, raw in records.items():
            item = tarfile.TarInfo(name)
            item.size, item.mode, item.uid, item.gid = len(raw), 0o600, 1001, 1001
            target.addfile(item, io.BytesIO(raw))
    return output.getvalue(), retired_count, len(records)


def archive_stat(docker, container_id, location, directory):
    if not re.fullmatch(r"[a-f0-9]{64}", container_id) or location not in (ROOT, UPLOAD_ROOT, UPLOAD_ROOT + "/ledger.json"):
        raise RuntimeError("exact Docker archive path required")
    request = (f"HEAD /containers/{container_id}/archive?path={location} HTTP/1.1\r\n"
               "Host: docker\r\nConnection: close\r\n\r\n").encode()
    response = docker("system", "dial-stdio", data=request)
    header, separator, body = response.partition(b"\r\n\r\n")
    if not separator or body or len(header) > 65536:
        raise RuntimeError("invalid Docker archive HEAD response")
    lines = header.decode("latin1").split("\r\n")
    status_line = re.fullmatch(r"HTTP/1\.[01] ([0-9]{3})(?: [^\r\n]*)?", lines[0])
    if not status_line:
        raise RuntimeError("invalid Docker archive HEAD status")
    status = status_line.group(1)
    if status == "404":
        return None
    if status != "200":
        raise RuntimeError("Docker archive stat unavailable")
    values = [line.partition(":")[2].strip() for line in lines[1:] if line.lower().startswith("x-docker-container-path-stat:")]
    if len(values) != 1:
        raise RuntimeError("exact Docker archive stat required")
    try:
        metadata = json.loads(base64.b64decode(values[0], validate=True))
    except (ValueError, UnicodeError) as error:
        raise RuntimeError("invalid Docker archive stat metadata") from error
    # Docker HEAD exposes Go FileMode but not UID. The consumer independently
    # requires a canonical UID1001/private root before it reads this proof.
    mode = metadata.get("mode") if isinstance(metadata, dict) else None
    expected_mode = (0x80000000 | 0o700) if directory else 0o600
    if type(mode) is not int or mode != expected_mode or metadata.get("linkTarget") or metadata.get("name") != location.rsplit("/", 1)[1]:
        raise RuntimeError("private nonsymlink Docker archive path required")
    return metadata


def legacy_upload_proof(docker, proof):
    if archive_stat(docker, proof["id"], UPLOAD_ROOT, directory=True) is None:
        return None, 0
    if archive_stat(docker, proof["id"], UPLOAD_ROOT + "/ledger.json", directory=False) is None:
        return None, 0
    archive = docker("cp", proof["id"] + ":" + UPLOAD_ROOT + "/ledger.json", "-")
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:*") as source:
        members = source.getmembers()
        if len(members) != 1:
            raise RuntimeError("only exact upload ledger may be captured")
        item = members[0]
        if item.name != "ledger.json" or not item.isfile() or item.uid != 1001 or item.mode & 0o777 != 0o600 or item.size > 1048576:
            raise RuntimeError("invalid private upload ledger")
        ledger = json.load(source.extractfile(item))
    if not isinstance(ledger, list) or len(ledger) > 128:
        raise RuntimeError("legacy upload ledger cardinality rejected")
    entries = []
    for entry in ledger:
        if not isinstance(entry, dict) or not re.fullmatch(r"[a-f0-9-]{36}", entry.get("id", "")) or type(entry.get("bytes")) is not int or not 0 <= entry["bytes"] <= 1024**3 or type(entry.get("count")) is not int or not 1 <= entry["count"] <= 20:
            raise RuntimeError("invalid legacy upload entry")
        if entry.get("lifetime") is None:
            entries.append(entry)
    if not entries:
        return None, 0
    raw = json.dumps({"version": 1, "authority": "docker-stopped-container", "engineId": proof["engine"],
                      "containerId": proof["id"], "startedAt": proof["started"], "finishedAt": proof["finished"], "entries": entries}).encode()
    if len(raw) > 1048576:
        raise RuntimeError("legacy proof size bound")
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as target:
        item = tarfile.TarInfo("legacy-container-retired.json")
        item.size, item.mode, item.uid, item.gid = len(raw), 0o600, 1001, 1001
        target.addfile(item, io.BytesIO(raw))
    return output.getvalue(), len(entries)


def reconcile(target, labels, docker=command):
    before = snapshot(docker, target, labels)
    if before is None:
        return {"retired": 0, "reason": "never-started-container"}
    # Only the exact archive HEAD 404 establishes absence, never CLI stderr.
    # A later cp failure is an error, even if the path disappeared after HEAD.
    metadata = archive_stat(docker, before["id"], ROOT, directory=True)
    archive = docker("cp", before["id"] + ":" + ROOT, "-") if metadata is not None else None
    output, count, entries = receipts(archive, before) if archive is not None else (None, 0, 0)
    legacy_output, legacy_count = legacy_upload_proof(docker, before)
    if snapshot(docker, before["id"], labels) != before:
        raise RuntimeError("container generation changed during lifetime capture")
    # Plain copies: each proof tar already carries UID/GID 1001 and mode 0600,
    # which Docker applies as written. `cp -a` would instead look up the
    # container's named user (slice) in the host user database on Docker 20.10
    # (Debian 12 docker.io) and fail for every stopped slice with a proof.
    if entries:
        docker("cp", "-", before["id"] + ":" + ROOT, data=output)
    if legacy_output:
        # This complete snapshot replaces earlier proof only with all currently
        # retained unbound entries. Partial writes fail parsing, never reclaim.
        docker("cp", "-", before["id"] + ":" + UPLOAD_ROOT, data=legacy_output)
    if snapshot(docker, before["id"], labels) != before:
        raise RuntimeError("container generation changed during proof delivery")
    return {**before, "retired": count, "legacyEntries": legacy_count}


if __name__ == "__main__":
    try:
        labels = dict(zip(("io.chariox.slice.id", "io.chariox.slice.owner-kernel-id", "io.chariox.slice.owner-machine-id"), sys.argv[2:]))
        if len(labels) != 3 or len(sys.argv) != 5:
            raise RuntimeError("container and exact slice/kernel/machine ownership arguments required")
        print(json.dumps(reconcile(sys.argv[1], labels)))
    except Exception as error:
        print("stopped browser reconciliation: " + str(error), file=sys.stderr)
        sys.exit(1)

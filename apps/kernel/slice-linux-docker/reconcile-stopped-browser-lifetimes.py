#!/usr/bin/env python3
"""Host Docker authority: retire only browser records captured after container exit."""
import io
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
LIMIT = 2 * 1024 * 1024


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
            if args[0] == "cp" and len(args) == 3 and args[2] == "-":
                container, location = args[1].split(":", 1)
                expected = f"Error response from daemon: Could not find the file {location.rstrip('/.')} in container {container}"
                if errors.decode().strip() == expected:
                    raise FileNotFoundError("exact Docker archive path absent")
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
                started = datetime.fromisoformat(existing["startedAt"].replace("Z", "+00:00"))
                finished = datetime.fromisoformat(existing["finishedAt"].replace("Z", "+00:00"))
                current_finished = datetime.fromisoformat(proof["finished"].replace("Z", "+00:00"))
                same = same and started <= finished <= current_finished and started.utcoffset() is not None
            except (ValueError, TypeError, KeyError, AttributeError):
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


def reconcile(target, labels, docker=command):
    before = snapshot(docker, target, labels)
    if before is None:
        return {"retired": 0, "reason": "never-started-container"}
    # Docker cp missing-path errors cannot establish absence. Inspect the stopped
    # container's exact path stat through Docker's archive API via cp; fail closed.
    try:
        archive = docker("cp", before["id"] + ":" + ROOT, "-")
    except FileNotFoundError:
        if snapshot(docker, before["id"], labels) != before:
            raise RuntimeError("container changed during absent-path observation")
        return {**before, "retired": 0, "reason": "no-lifecycle-record-directory"}
    output, count, entries = receipts(archive, before)
    if snapshot(docker, before["id"], labels) != before:
        raise RuntimeError("container generation changed during lifetime capture")
    if entries:
        docker("cp", "-a", "-", before["id"] + ":" + ROOT, data=output)
    return {**before, "retired": count}


if __name__ == "__main__":
    try:
        labels = dict(zip(("io.chariox.slice.id", "io.chariox.slice.owner-kernel-id", "io.chariox.slice.owner-machine-id"), sys.argv[2:]))
        if len(labels) != 3 or len(sys.argv) != 5:
            raise RuntimeError("container and exact slice/kernel/machine ownership arguments required")
        print(json.dumps(reconcile(sys.argv[1], labels)))
    except Exception as error:
        print("stopped browser reconciliation: " + str(error), file=sys.stderr)
        sys.exit(1)

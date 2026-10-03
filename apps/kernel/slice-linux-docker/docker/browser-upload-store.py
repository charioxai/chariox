#!/usr/bin/env python3
"""One bounded quota transaction, wholly owned by a kernel-held flock."""
import fcntl
from datetime import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import time

spec = importlib.util.spec_from_file_location("browser_lifecycle", Path(__file__).with_name("browser-lifecycle.py"))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)
MAX_BYTES = 1024 * 1024 * 1024
MAX_FILES = 128
BOUNDARY_FIELDS = {"version", "authority", "engineId", "containerId", "startedAt", "finishedAt"}


def lock_quota(stream):
    deadline = time.monotonic() + 1
    while True:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return
        except BlockingIOError:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise
            time.sleep(min(0.02, remaining))


def process_state(owner):
    current = lifecycle.identity(os.getpid())
    if owner["boot"] != current["boot"]:
        return "dead"
    if owner["namespace"] != current["namespace"]:
        return "unknown"
    try:
        observed = lifecycle.identity(owner["pid"])
        if observed["start"] != owner["start"]:
            return "dead"
        if observed != owner:
            return "unknown"
        status = Path(f"/proc/{owner['pid']}/stat").read_text().rpartition(")")[2].split()[0]
        return "dead" if status == "Z" else "alive"
    except (FileNotFoundError, ProcessLookupError):
        return "dead"


def browser_owner(pid, port):
    if not isinstance(pid, int) or pid < 1 or not isinstance(port, int) or not 0 < port < 65536:
        return None
    # Match the actual local listening socket, not a PID reported by a proxy.
    sockets = set()
    for protocol in ("tcp", "tcp6"):
        for line in Path(f"/proc/net/{protocol}").read_text().splitlines()[1:]:
            parts = line.split()
            if int(parts[1].split(":")[1], 16) == port and parts[3] == "0A":
                sockets.add(f"socket:[{parts[9]}]")
    try:
        if not any(os.readlink(fd) in sockets for fd in Path(f"/proc/{pid}/fd").iterdir()):
            return None
        for path in lifecycle.directory().glob("*.json"):
            if not re.fullmatch(r"[a-f0-9]{32}\.json", path.name):
                continue
            record = lifecycle.read_json(path)
            if (record.get("browser") or {}).get("pid") == pid and process_state(record["browser"]) == "alive" and process_state(record["supervisor"]) == "alive":
                return record
    except (FileNotFoundError, ProcessLookupError):
        return None
    return None


def retired(record):
    if not record:
        return False
    try:
        receipt = lifecycle.read_json(lifecycle.directory() / f"{record['instance']}.retired.json")
        return receipt == record
    except FileNotFoundError:
        return False


def boundary_timestamp(value):
    if not isinstance(value, str):
        raise ValueError("timestamp must be a string")
    match = re.fullmatch(r"([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2})(?:\.([0-9]{1,9}))?Z", value)
    if not match:
        raise ValueError("invalid Docker timestamp")
    # datetime validates the calendar; retain all Docker nanoseconds for order.
    return datetime.fromisoformat(match[1]), int((match[2] or "").ljust(9, "0"))


def valid_container_boundary(proof):
    valid = (isinstance(proof, dict) and BOUNDARY_FIELDS <= proof.keys()
            and type(proof["version"]) is int and proof["version"] == 1 and proof["authority"] == "docker-stopped-container"
            and isinstance(proof["engineId"], str) and re.fullmatch(r"[a-zA-Z0-9:_-]{8,128}", proof["engineId"]) is not None
            and isinstance(proof["containerId"], str) and re.fullmatch(r"[a-f0-9]{64}", proof["containerId"]) is not None
            )
    if not valid:
        return False
    try:
        return boundary_timestamp(proof["startedAt"]) <= boundary_timestamp(proof["finishedAt"])
    except ValueError:
        return False


def legacy_retirements(root):
    try:
        proof = lifecycle.read_json(root / "legacy-container-retired.json", 1048576)
    except FileNotFoundError:
        return []
    if not valid_container_boundary(proof) or set(proof) != BOUNDARY_FIELDS | {"entries"}:
        raise RuntimeError("invalid legacy container retirement proof")
    entries = proof["entries"]
    if not isinstance(entries, list) or len(entries) > MAX_FILES or any(
            not isinstance(entry, dict) or entry.get("lifetime") is not None
            or entry.get("phase") not in ("preparing", "exposed") for entry in entries):
        raise RuntimeError("invalid captured legacy uploads")
    return entries


def container_retired(entry):
    record = entry.get("lifetime")
    if not record or not retired(record):
        return False
    owner, supervisor = entry.get("owner") or {}, record.get("supervisor") or {}
    if any(key not in owner or key not in supervisor or owner[key] != supervisor[key]
           for key in ("boot", "namespace", "uid")):
        return False
    try:
        proof = lifecycle.read_json(lifecycle.directory() / f"{record['instance']}.container-retired.json")
    except FileNotFoundError:
        return False
    # Only the normal host Docker lifecycle writes this companion after proving
    # the entire container exited. Graceful browser retirement does not prove
    # that a controller still preparing files has stopped.
    return (valid_container_boundary(proof) and set(proof) == BOUNDARY_FIELDS | {"lifetime"}
            and proof["lifetime"] == record)


def prune_receipts(ledger):
    root = lifecycle.directory()
    retained = {entry["lifetime"]["instance"] for entry in ledger if entry.get("lifetime")}
    for receipt in root.glob("*.retired.json"):
        record = lifecycle.read_json(receipt)
        if record["instance"] in retained:
            continue
        try:
            if lifecycle.read_json(lifecycle.current_path(root, record["profile"])) == record:
                continue
        except FileNotFoundError:
            continue
        receipt.unlink()
        (root / f"{record['instance']}.container-retired.json").unlink(missing_ok=True)
        (root / f"{record['instance']}.json").unlink(missing_ok=True)


def transact(root, request):
    root.mkdir(mode=0o700, exist_ok=True)
    info = root.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o777 != 0o700 or root.resolve() != root:
        raise RuntimeError("upload staging root must be canonical, private and user-owned")
    if (root / "lock").exists():
        raise RuntimeError("legacy upload lock has no verifiable owner; reconciliation required")
    lock = os.open(root / "quota.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(lock, "w") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o777 != 0o600:
            raise RuntimeError("invalid upload quota lock")
        lock_quota(stream)
        # No earlier mutator can still own a transaction once flock succeeds.
        for temporary in root.glob("record-*"):
            info = temporary.lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise RuntimeError("invalid interrupted quota transaction")
            temporary.unlink()
        try:
            ledger = lifecycle.read_json(root / "ledger.json", 1048576)
        except FileNotFoundError:
            if any(path.name != "quota.lock" for path in root.iterdir()):
                raise RuntimeError("upload staging ledger is missing with retained files")
            ledger = []
        if not isinstance(ledger, list) or len(ledger) > 128:
            raise RuntimeError("invalid upload ledger")
        for entry in ledger:
            if not isinstance(entry, dict) or not re.fullmatch(r"[a-f0-9-]{36}", entry.get("id", "")) or not isinstance(entry.get("bytes"), int) or not 0 <= entry["bytes"] <= MAX_BYTES or not isinstance(entry.get("count"), int) or not 1 <= entry["count"] <= 20:
                raise RuntimeError("invalid upload ledger entry")
        legacy = legacy_retirements(root)
        removable = [entry for entry in ledger if entry in legacy or
                     (entry.get("phase") == "preparing" and (process_state(entry["owner"]) == "dead" or container_retired(entry)))
                     or (entry.get("phase") == "exposed" and retired(entry.get("lifetime")))]
        for entry in removable:
            shutil.rmtree(root / entry["id"], ignore_errors=False) if (root / entry["id"]).exists() else None
            ledger.remove(entry)
        action = request["action"]
        result = None
        if action == "reserve":
            entry = request["entry"]
            if not re.fullmatch(r"[a-f0-9-]{36}", entry["id"]) or not isinstance(entry["bytes"], int) or not 0 <= entry["bytes"] <= MAX_BYTES or not isinstance(entry["count"], int) or not 1 <= entry["count"] <= 20:
                raise RuntimeError("invalid upload reservation")
            if len(ledger) >= 128 or sum(item["bytes"] for item in ledger) + entry["bytes"] > min(request["maximumBytes"], MAX_BYTES) or sum(item["count"] for item in ledger) + entry["count"] > min(request["maximumFiles"], MAX_FILES):
                raise RuntimeError("upload staging quota is full; browser retirement proof is required")
            lifetime = browser_owner(request.get("browserPid"), request.get("browserPort"))
            if lifetime is None:
                raise RuntimeError("upload requires an owned browser lifetime")
            entry.update(phase="preparing", owner=lifecycle.identity(os.getppid()), lifetime=lifetime)
            ledger.append(entry)
        elif action in ("expose", "discard"):
            entry = next((item for item in ledger if item["id"] == request["id"]), None)
            if entry is None or entry.get("owner") != lifecycle.identity(os.getppid()):
                raise RuntimeError("upload reservation owner changed")
            if action == "expose":
                if not entry.get("lifetime"):
                    raise RuntimeError("upload requires an owned browser lifetime")
                if retired(entry.get("lifetime")):
                    raise RuntimeError("browser retired before upload dispatch")
                entry["phase"] = "exposed"
            elif entry.get("phase") == "preparing":
                shutil.rmtree(root / entry["id"]) if (root / entry["id"]).exists() else None
                ledger.remove(entry)
        elif action != "reap":
            raise RuntimeError("unknown upload store operation")
        lifecycle.write_json(root / "ledger.json", ledger)
        if not any(entry in legacy for entry in ledger):
            (root / "legacy-container-retired.json").unlink(missing_ok=True)
        prune_receipts(ledger)
        return result


if __name__ == "__main__":
    try:
        request = json.loads(sys.stdin.buffer.read(65537))
        result = transact(Path(sys.argv[1]), request)
        print(json.dumps(result))
    except Exception as error:
        print(f"upload staging transaction failed: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(1)

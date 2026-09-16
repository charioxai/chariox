#!/usr/bin/env python3
"""Private slice-owned Selkies lifecycle. No Room or input authority lives here."""

import fcntl
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

import psutil


def state_directory():
    base = Path(os.environ.get("XDG_RUNTIME_DIR", f"/tmp/chariox-slice-{os.getuid()}"))
    directory = base / "selkies"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError("Selkies state must be a private directory owned by the slice user")
    return directory


def read_state(directory):
    try:
        return json.loads((directory / "process.json").read_text())
    except FileNotFoundError:
        return None


def write_state(directory, record):
    descriptor, filename = tempfile.mkstemp(prefix="process-", suffix=".tmp", dir=directory)
    try:
        with os.fdopen(descriptor, "w") as stream:
            json.dump(record, stream)
        os.replace(filename, directory / "process.json")
    finally:
        Path(filename).unlink(missing_ok=True)


def process_start_identity(pid):
    # Linux start ticks and boot ID do not move when the VM wall clock is
    # corrected. psutil.create_time() is derived from wall-clock boot time.
    boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    fields = Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()
    return {"boot_id": boot_id, "start_ticks": int(fields[19])}


def process_record(process):
    return {"pid": process.pid, "created": process.create_time(),
            **process_start_identity(process.pid)}


def process_key(record):
    if "boot_id" in record or "start_ticks" in record:
        return (record["pid"], record.get("boot_id"), record.get("start_ticks"))
    return (record["pid"], record["created"])


def owned_process(record):
    if record is None:
        return None
    try:
        process = psutil.Process(record["pid"])
        if "boot_id" in record or "start_ticks" in record:
            identity = process_start_identity(process.pid)
            same_process = (identity["boot_id"] == record.get("boot_id")
                            and identity["start_ticks"] == record.get("start_ticks"))
        else:
            # Older records retain the strict check. Never adopt a legacy PID
            # by adding a timestamp tolerance or trusting a health response.
            same_process = process.create_time() == record["created"]
        if (same_process
                and process.uids().real == os.getuid()
                and process.status() != psutil.STATUS_ZOMBIE):
            return process
    except (psutil.NoSuchProcess, psutil.AccessDenied, OSError, ValueError, IndexError):
        pass
    return None


def endpoint(record):
    return f"http://127.0.0.1:{record['port']}"


def healthy(record):
    if owned_process(record) is None:
        return False
    try:
        with urllib.request.urlopen(endpoint(record) + "/api/health", timeout=1) as response:
            return response.status == 200 and response.read(16) == b"OK"
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


def public_status(record):
    available = healthy(record)
    result = {"available": available, "kind": "selkies"}
    if available:
        result.update(pid=record["pid"], endpoint=endpoint(record) + "/")
    return result


def wait_until_not_owned(record, timeout):
    deadline = time.monotonic() + timeout
    while owned_process(record) is not None:
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.05)
    return True


def stop(directory):
    record = read_state(directory)
    process = owned_process(record)
    forced = False
    if process is not None:
        process.terminate()
        if not wait_until_not_owned(record, 10):
            forced = True
            process = owned_process(record)
            if process is not None:
                process.kill()
            if not wait_until_not_owned(record, 5):
                raise RuntimeError("Selkies did not stop after forced termination")
    (directory / "process.json").unlink(missing_ok=True)
    return {"stopped": True, "forced": forced}


def start(directory, *, port=None, display=None):
    port = int(port if port is not None else os.environ.get("CHARIOX_SLICE_NOVNC_PORT", "6080"))
    if not 1 <= port <= 65535:
        raise ValueError("invalid display port")
    display = display if display is not None else os.environ.get("DISPLAY", ":99")
    previous = read_state(directory)
    if owned_process(previous) is not None:
        if previous["port"] != port or previous["display"] != display:
            raise RuntimeError("stop the existing streamer before changing its display or port")
        if not healthy(previous):
            raise RuntimeError("existing streamer is unhealthy; stop it before restarting")
        return public_status(previous)

    token = secrets.token_urlsafe(32)
    environment = {**os.environ, "DISPLAY": display, "SELKIES_MASTER_TOKEN": token}
    command = [
        os.environ.get("CHARIOX_SLICE_SELKIES_BIN", "/opt/chariox-selkies/bin/selkies"),
        "--addr=127.0.0.1", f"--port={port}", "--mode=websockets",
        "--encoder=h264enc", "--use-cpu=true|locked", "--framerate=30",
        "--enable-https=false", "--enable-basic-auth=false",
        "--enable-resize=false|locked", "--enable-collab=false|locked",
        "--audio-enabled=false|locked", "--microphone-enabled=false|locked",
        "--webcam-enabled=false|locked", "--gamepad-enabled=false|locked",
        "--command-enabled=false|locked", "--file-transfers=none",
        "--enable-clipboard=false",
    ]
    log_descriptor = os.open(directory / "streamer.log", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    child = None
    try:
        with os.fdopen(log_descriptor, "w") as log:
            child = subprocess.Popen(command, env=environment, stdin=subprocess.DEVNULL,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        process = psutil.Process(child.pid)
        record = {**process_record(process), "port": port,
                  "display": display, "master_token": token}
        write_state(directory, record)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError(f"Selkies exited with {child.returncode}; inspect {directory / 'streamer.log'}")
            if healthy(record):
                # Verify this process's credential and open the configuration
                # gate with NO viewers. The kernel provisions scoped viewers later.
                request = urllib.request.Request(
                    endpoint(record) + "/api/tokens", data=b"{}", method="POST",
                    headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
                )
                with urllib.request.urlopen(request, timeout=2) as response:
                    if response.status != 200:
                        raise RuntimeError("Selkies rejected its private control credential")
                return public_status(record)
            time.sleep(0.1)
        raise RuntimeError("Selkies did not become healthy within 15 seconds")
    except BaseException:
        if child is not None:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
            (directory / "process.json").unlink(missing_ok=True)
        raise


def main():
    allow_forced = sys.argv[1:] == ["stop", "--allow-forced"]
    if not allow_forced and (len(sys.argv) != 2 or sys.argv[1] not in ("start", "status", "stop")):
        raise ValueError("usage: slice-selkies.py start|status|stop [--allow-forced for stop only]")
    directory = state_directory()
    descriptor = os.open(directory / "lifecycle.lock", os.O_RDWR | os.O_CREAT, 0o600)
    with os.fdopen(descriptor, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        action = sys.argv[1]
        if action == "start":
            result = start(directory)
        elif action == "stop":
            result = stop(directory)
        else:
            result = public_status(read_state(directory))
        print(json.dumps(result))
        return 0 if result.get("available", True) and (allow_forced or not result.get("forced", False)) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)

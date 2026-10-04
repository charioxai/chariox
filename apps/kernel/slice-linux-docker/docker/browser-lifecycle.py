#!/usr/bin/env python3
"""Linux Chromium child-tree ownership; never infer retirement from CDP loss."""
import ctypes
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid


def identity(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()
    return {"pid": pid, "start": fields[19], "group": int(fields[2]), "session": int(fields[3]),
            "uid": Path(f"/proc/{pid}").stat().st_uid,
            "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
            "namespace": os.readlink("/proc/self/ns/pid")}


def same_process(record):
    try:
        state = Path(f"/proc/{record['pid']}/stat").read_text().rpartition(")")[2].split()[0]
        return identity(record["pid"]) == record and state != "Z"
    except (FileNotFoundError, ProcessLookupError):
        return False


def directory():
    root = Path(os.environ.get("CHARIOX_BROWSER_LIFECYCLE_ROOT", f"/tmp/chariox-browser-lifecycle-{os.getuid()}"))
    root.mkdir(mode=0o700, exist_ok=True)
    info = root.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o777 != 0o700 or root.resolve() != root:
        raise RuntimeError("browser lifecycle directory must be canonical, private and user-owned")
    return root


def write_json(path, value):
    fd, temporary = tempfile.mkstemp(prefix="record-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        Path(temporary).unlink(missing_ok=True)


def read_json(path, maximum_bytes=4096):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd) as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o777 != 0o600 or info.st_size > maximum_bytes:
            raise RuntimeError("invalid browser lifecycle record")
        return json.load(stream)


def current_path(root, profile):
    return root / (hashlib.sha256(profile.encode()).hexdigest() + ".json")


def profile_processes(profile, retired_supervisor=None):
    result = []
    for path in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            pid = int(path.parent.name)
            # The launcher carries the browser's argv but is not a browser.
            if pid == os.getpid():
                continue
            # A verified retirement receipt proves this owner's children exited.
            # The owner itself may still be finishing upload cleanup.
            if retired_supervisor and pid == retired_supervisor["pid"] and same_process(retired_supervisor):
                continue
            if f"--user-data-dir={profile}".encode() in path.read_bytes().split(b"\0"):
                result.append(pid)
        except (FileNotFoundError, ProcessLookupError):
            continue
    return result


def reap_children():
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
            if pid == 0:
                return False
        except ChildProcessError:
            return True


def descendants():
    parents = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / "stat").read_text().rpartition(")")[2].split()
            parents[int(entry.name)] = int(fields[1])
        except (FileNotFoundError, ProcessLookupError):
            continue
    owned = {os.getpid()}
    while True:
        found = {pid for pid, parent in parents.items() if parent in owned}
        if found <= owned:
            return owned - {os.getpid()}
        owned.update(found)


def signal_owned_children(sig):
    # A pidfd cannot target a reused PID. Recheck membership after opening it.
    for pid in descendants():
        try:
            fd = os.pidfd_open(pid)
            try:
                if pid in descendants():
                    signal.pidfd_send_signal(fd, sig)
            finally:
                os.close(fd)
        except (ProcessLookupError, FileNotFoundError):
            pass


def reap_uploads():
    root = Path(tempfile.gettempdir()).resolve() / f"chariox-browser-uploads-{os.getuid()}"
    subprocess.run([sys.executable, str(Path(__file__).with_name("browser-upload-store.py")), str(root)],
                   input='{"action":"reap"}', text=True, stdout=subprocess.DEVNULL,
                   timeout=3, check=True)


def lock_launch(fd):
    deadline = time.monotonic() + 1
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return
        except BlockingIOError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.02)


def supervise(root, instance, profile, log, command):
    if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        raise RuntimeError("cannot own Chromium descendant lifetime")
    stopping = []
    signal.signal(signal.SIGTERM, lambda _sig, _frame: stopping.append(time.monotonic()) if not stopping else None)
    owner = identity(os.getpid())
    with open(log, "ab", buffering=0) as output:
        browser = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output, stderr=output, start_new_session=True)
    try:
        browser_identity = identity(browser.pid)
    except FileNotFoundError:
        browser_identity = None
    record = {"version": 1, "instance": instance, "profile": profile, "supervisor": owner, "browser": browser_identity}
    write_json(root / f"{instance}.json", record)
    while not reap_children():
        if stopping:
            signal_owned_children(signal.SIGKILL if time.monotonic() - stopping[0] >= 3 else signal.SIGTERM)
        time.sleep(0.05 if stopping else 0.25)
    # ECHILD proves even setsid/double-fork descendants have finished. A killed
    # supervisor never writes this receipt, so ambiguous deaths retain data.
    write_json(root / f"{instance}.retired.json", record)
    reap_uploads()


def start(profile, log, command):
    root = directory()
    lock = os.open(root / "launch.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock, "w") as stream:
        lock_launch(stream)
        pointer = current_path(root, profile)
        if pointer.exists():
            previous = read_json(pointer)
            if same_process(previous["supervisor"]):
                raise RuntimeError("an owned browser lifetime already exists for this profile")
            if not (root / f"{previous['instance']}.retired.json").exists() or read_json(root / f"{previous['instance']}.retired.json") != previous:
                raise RuntimeError("previous browser lifetime requires reconciliation")
        if profile_processes(profile):
            raise RuntimeError("profile has an unowned browser process; refusing to replace its lifetime")
        if len(list(root.glob("*.retired.json"))) >= 128:
            raise RuntimeError("browser lifecycle receipt quota requires reconciliation")
        instance = uuid.uuid4().hex
        child = subprocess.Popen([sys.executable, __file__, "supervise", str(root), instance, profile, log, *command],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        deadline = time.monotonic() + 5
        while not (root / f"{instance}.json").exists():
            if child.poll() is not None or time.monotonic() >= deadline:
                # Do not kill an unrecorded tree. It owns retirement even when
                # startup reporting fails, and its supervisor remains isolated.
                raise RuntimeError("browser lifetime did not acknowledge startup")
            time.sleep(0.02)
        record = read_json(root / f"{instance}.json")
        write_json(pointer, record)
        try:
            reap_uploads()
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            # The browser is already owned and alive. Deferred garbage
            # collection cannot turn successful startup into a teardown signal.
            # Quota remains retained and enforced by the upload store.
            print("browser upload cleanup deferred; retained quota remains charged", file=sys.stderr)
        print(json.dumps(record))


def stop(profile):
    root = directory()
    lock = os.open(root / "launch.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        lock_launch(lock)
        stop_locked(root, profile)
    finally:
        os.close(lock)


def stop_locked(root, profile, settlement_timeout=12):
    try:
        record = read_json(current_path(root, profile))
    except FileNotFoundError:
        if profile_processes(profile):
            raise RuntimeError("running browser has no owned lifetime record; refusing pattern-based termination")
        return
    receipt = root / f"{record['instance']}.retired.json"
    if not receipt.exists():
        owner = record["supervisor"]
        if not same_process(owner):
            raise RuntimeError("browser supervisor identity is gone without retirement proof")
        # MP-08/MP-10/MP-11: a CDP close acknowledgement precedes profile
        # flush and descendant retirement. Share the existing three-second
        # close budget with settlement before falling back to owned TERM.
        close_deadline = time.monotonic() + 3
        try:
            closed = subprocess.run(["node", str(Path(__file__).with_name("browser-cdp.mjs")), "close-browser", "--owned-profile", profile],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3, check=False)
            while closed.returncode == 0 and not receipt.exists() and time.monotonic() < close_deadline:
                time.sleep(0.05)
        except subprocess.TimeoutExpired:
            pass
        if receipt.exists():
            if read_json(receipt) != record:
                raise RuntimeError("browser retirement proof does not match its owner")
            if profile_processes(profile, record["supervisor"]):
                raise RuntimeError("an unowned browser still uses the profile")
            reap_uploads()
            print(json.dumps(record))
            return
        fd = os.pidfd_open(owner["pid"])
        try:
            if not same_process(owner):
                raise RuntimeError("browser supervisor identity changed")
            signal.pidfd_send_signal(fd, signal.SIGTERM)
        finally:
            os.close(fd)
        deadline = time.monotonic() + settlement_timeout
        while not receipt.exists():
            if time.monotonic() >= deadline:
                raise RuntimeError("browser descendants did not settle; upload data remains retained")
            time.sleep(0.05)
    if read_json(receipt) != record:
        raise RuntimeError("browser retirement proof does not match its owner")
    if profile_processes(profile, record["supervisor"]):
        raise RuntimeError("an unowned browser still uses the profile")
    reap_uploads()
    print(json.dumps(record))


if __name__ == "__main__":
    try:
        if sys.platform != "linux" or not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
            raise RuntimeError("browser lifecycle requires Linux pidfd support")
        action, *args = sys.argv[1:]
        if action == "start":
            start(args[0], args[1], args[2:])
        elif action == "stop":
            stop(args[0])
        elif action == "supervise":
            supervise(Path(args[0]), args[1], args[2], args[3], args[4:])
        elif action == "verify":
            spec = importlib.util.spec_from_file_location("upload_store", Path(__file__).with_name("browser-upload-store.py"))
            store = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(store)
            if store.browser_owner(int(args[1]), int(args[2])) != read_json(current_path(directory(), args[0])):
                raise RuntimeError("browser process does not own the requested lifetime")
        else:
            raise RuntimeError("unknown browser lifecycle command")
    except Exception as error:
        print(f"browser lifecycle: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(1)

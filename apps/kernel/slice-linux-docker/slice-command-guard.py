#!/usr/bin/env python3
"""Own a provisioner subprocess group until its output and process settle."""
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import stat
import subprocess
import sys
import time


def progress_timeout_seconds():
    policy = json.loads(Path(__file__).with_name("home-archive-policy.json").read_text())
    value = policy.get("progressTimeoutMs")
    if (set(policy) != {"schemaVersion", "minimumFreeBytes", "progressTimeoutMs"}
            or type(policy.get("schemaVersion")) is not int or policy["schemaVersion"] != 1
            or type(policy.get("minimumFreeBytes")) is not int
            or not 0 <= policy["minimumFreeBytes"] <= 9007199254740991
            or type(value) is not int or not 0 < value <= 2147483647):
        raise ValueError("home archive progress timeout must be a positive bounded integer")
    return value / 1000


def reset_child_signals():
    signal.signal(signal.SIGTERM, signal.SIG_DFL)
    signal.signal(signal.SIGINT, signal.SIG_DFL)


def stop_orphaned_anchor(parent_pid):
    if os.getppid() == parent_pid:
        return
    # An abruptly lost supervisor cannot release this still-occupied group.
    os.killpg(os.getpgrp(), signal.SIGTERM)
    time.sleep(0.1)
    os.killpg(os.getpgrp(), signal.SIGKILL)


def finish_worker(status, progress_fd, parent_pid):
    os.write(progress_fd, ("s " + str(status) + "\n").encode("ascii"))
    # The result does not relinquish group ownership. The supervisor settles
    # every remaining group member before reaping this anchor.
    while True:
        stop_orphaned_anchor(parent_pid)
        time.sleep(0.025)


def command_worker(command, progress_fd, parent_pid):
    # This anchor remains the occupied process-group leader even if the actual
    # command exits while one of its descendants retains an output pipe.
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    stop_orphaned_anchor(parent_pid)
    child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             preexec_fn=reset_child_signals)
    try:
        with selectors.DefaultSelector() as ready:
            ready.register(child.stdout, selectors.EVENT_READ, sys.stdout.buffer)
            ready.register(child.stderr, selectors.EVENT_READ, sys.stderr.buffer)
            while ready.get_map():
                stop_orphaned_anchor(parent_pid)
                for key, _ in ready.select(0.025):
                    data = os.read(key.fd, 65536)
                    if not data:
                        ready.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    key.data.write(data)
                    key.data.flush()
                    os.write(progress_fd, b"p\n")
        while child.poll() is None:
            stop_orphaned_anchor(parent_pid)
            time.sleep(0.025)
        status = child.returncode if child.returncode >= 0 else 128 - child.returncode
        finish_worker(status, progress_fd, parent_pid)
    except (OSError, ValueError):
        # Keep ownership until the supervisor receives failure and settles this
        # group; exiting here could leave a command after a broken output pipe.
        try:
            os.write(progress_fd, b"e\n")
        except OSError:
            pass
        while True:
            stop_orphaned_anchor(parent_pid)
            time.sleep(0.025)


def digest_worker(path, progress_fd, parent_pid):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    digest = hashlib.sha256()
    stop_orphaned_anchor(parent_pid)
    with open(path, "rb", buffering=0) as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode):
            raise ValueError("archive is not a regular file")
        while True:
            stop_orphaned_anchor(parent_pid)
            data = source.read(65536)
            if not data:
                break
            digest.update(data)
            os.write(progress_fd, b"p\n")
        after = os.fstat(source.fileno())
        identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        if identity(before) != identity(after):
            raise ValueError("archive changed during hashing")
    print(digest.hexdigest(), flush=True)
    finish_worker(0, progress_fd, parent_pid)


def source_digest_worker(repository, progress_fd, parent_pid):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    stop_orphaned_anchor(parent_pid)
    digest = hashlib.sha256()
    for raw in sys.stdin.buffer:
        stop_orphaned_anchor(parent_pid)
        path_bytes = raw[:-1] if raw.endswith(b"\n") else raw
        path = os.path.join(repository, os.fsdecode(path_bytes))
        # Preserve the shell's [[ -f ]] behavior, including symlink following
        # and skipping Git-quoted names that are not literal filesystem paths.
        if not os.path.isfile(path):
            continue
        file_digest = hashlib.sha256()
        with open(path, "rb", buffering=0) as source:
            while True:
                data = source.read(65536)
                if not data:
                    break
                file_digest.update(data)
                os.write(progress_fd, b"p\n")
        digest.update(path_bytes + b" " + file_digest.hexdigest().encode("ascii") + b"\n")
        os.write(progress_fd, b"p\n")
    print(digest.hexdigest(), flush=True)
    finish_worker(0, progress_fd, parent_pid)


def run_owned(worker_args, timeout_seconds=None, progress=False):
    progress_read, progress_write = os.pipe()
    os.set_blocking(progress_read, False)
    parent_pid = os.getppid()
    interrupted = []
    previous = {}
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        previous[signum] = signal.signal(signum, lambda received, frame: interrupted.append(received))
    child = None
    settled = completed = False
    pending = b""
    try:
        child = subprocess.Popen([sys.executable, __file__, *worker_args, str(progress_write), str(os.getpid())],
                                 start_new_session=True, pass_fds=(progress_write,))
        os.close(progress_write)
        progress_write = None
        last_progress = started = time.monotonic()
        with selectors.DefaultSelector() as ready:
            ready.register(progress_read, selectors.EVENT_READ)
            while True:
                status = child.poll()
                if status is not None:
                    settled = True
                    return status if status >= 0 else 128 - status
                if interrupted or os.getppid() != parent_pid:
                    return 128 + (interrupted[0] if interrupted else signal.SIGTERM)
                if timeout_seconds is not None:
                    since = last_progress if progress else started
                    if time.monotonic() - since >= timeout_seconds:
                        print("slice command exceeded its progress deadline" if progress else
                              "slice command timed out", file=sys.stderr)
                        return 124
                for _, _ in ready.select(0.025):
                    data = os.read(progress_read, 65536)
                    if not data:
                        ready.unregister(progress_read)
                        continue
                    pending += data
                    while b"\n" in pending:
                        record, pending = pending.split(b"\n", 1)
                        if record == b"e":
                            return 1
                        if record == b"p":
                            last_progress = time.monotonic()
                        elif record.startswith(b"s "):
                            status = int(record[2:])
                            if not 0 <= status <= 255:
                                raise ValueError("invalid command result")
                            completed = True
                            return status
                        else:
                            raise ValueError("invalid command progress")
                    if len(pending) > 32:
                        raise ValueError("invalid command progress")
    finally:
        # Never signal a group after its anchor has been reaped. On cancellation
        # keep the unreaped anchor until TERM/KILL and wait have settled the group.
        if child is not None and not settled:
            if completed:
                # The foreground command has settled; remaining group members
                # cannot be allowed to survive a successful control operation.
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
            else:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    child.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    child.wait()
        os.close(progress_read)
        if progress_write is not None:
            os.close(progress_write)
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def main(args):
    if not args:
        raise ValueError("missing slice command guard arguments")
    if args[0] == "_command":
        return command_worker(args[1:-2], int(args[-2]), int(args[-1]))
    if args[0] == "_digest":
        return digest_worker(args[1], int(args[-2]), int(args[-1]))
    if args[0] == "_source":
        return source_digest_worker(args[1], int(args[-2]), int(args[-1]))
    if args[0] == "digest-paths" and len(args) == 2:
        return run_owned(["_source", args[1]], progress_timeout_seconds(), progress=True)
    if args[0] == "digest" and len(args) == 2:
        return run_owned(["_digest", args[1]], progress_timeout_seconds(), progress=True)
    if args[0] == "run" and len(args) >= 4 and args[2] == "--":
        seconds = float(args[1])
        if not 0 < seconds <= 2147483647:
            raise ValueError("command timeout must be a positive bounded number")
        return run_owned(["_command", *args[3:]], seconds)
    if args[0] == "unbounded" and len(args) >= 3 and args[1] == "--":
        return run_owned(["_command", *args[2:]])
    raise ValueError("invalid slice command guard arguments")


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError) as error:
        # Do not include command arguments or payloads in failures.
        print("slice command guard failed: " + type(error).__name__, file=sys.stderr)
        sys.exit(1)

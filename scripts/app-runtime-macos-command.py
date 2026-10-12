"""Owned command-group leader for the monitored builder, never an App host.

FD3 is a private status/ack socket. Retain this leader until the JS parent has
observed completion and stopped its watcher, avoiding signals to reused PIDs.
"""
import json
import os
import select
import signal
import subprocess
import sys
sys.dont_write_bytecode = True
import time

# MP-11: load the one repository guard even under Python isolated mode.
import importlib.util
from pathlib import Path
_guard_path = Path(__file__).resolve().parent.parent / "apps/kernel/slice-linux-docker/owned_process_signals.py"
_guard_spec = importlib.util.spec_from_file_location("owned_process_signals", _guard_path)
_guard_module = importlib.util.module_from_spec(_guard_spec)
_guard_spec.loader.exec_module(_guard_module)
owned_signals = _guard_module.OwnedProcesses()


def stop_group():
    owned_signals.group(anchor_signal_handle, signal.SIGKILL)


def main():
    if os.getpid() != os.getpgrp() or len(sys.argv) < 2:
        raise ValueError("command owner must lead its own group")
    global anchor_signal_handle
    anchor_signal_handle = owned_signals.record(os.getpid(), watch=True)
    # A caught handler is reset on exec, so command children retain SIGTERM's
    # default behavior. Keep this owner until the parent kills/acknowledges it.
    signal.signal(signal.SIGTERM, lambda *_: None)
    signal.signal(signal.SIGINT, lambda *_: None)
    child = subprocess.Popen(sys.argv[1:], close_fds=True)
    sent = False
    deadline = time.monotonic() + 300 * 60 + 10
    while time.monotonic() < deadline:
        owned_signals.refresh()
        status = child.poll()
        if status is not None and not sent:
            os.write(3, (json.dumps({"status": status}) + "\n").encode("ascii"))
            sent = True
        readable, _, _ = select.select([3], [], [], 0.05)
        if readable:
            message = os.read(3, 2)
            if sent and status == 0 and message == b"!":
                return
            stop_group()  # EOF, malformed ack or a failed command.
    stop_group()


if __name__ == "__main__":
    try:
        main()
    except BaseException:
        stop_group()

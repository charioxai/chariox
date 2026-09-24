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
import time


def stop_group():
    os.killpg(os.getpid(), signal.SIGKILL)


def main():
    if os.getpid() != os.getpgrp() or len(sys.argv) < 2:
        raise ValueError("command owner must lead its own group")
    # A caught handler is reset on exec, so command children retain SIGTERM's
    # default behavior. Keep this owner until the parent kills/acknowledges it.
    signal.signal(signal.SIGTERM, lambda *_: None)
    signal.signal(signal.SIGINT, lambda *_: None)
    child = subprocess.Popen(sys.argv[1:], close_fds=True)
    sent = False
    deadline = time.monotonic() + 300 * 60 + 10
    while time.monotonic() < deadline:
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

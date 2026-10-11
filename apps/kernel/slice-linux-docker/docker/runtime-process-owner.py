#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: supervise one runtime launch and its owned descendants."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

sys.path[:0] = [str(Path(__file__).resolve().parent), str(Path(__file__).resolve().parent.parent)]
from owned_process_signals import OwnedProcesses, valid_id


def settle(owner, launch, sig):
    groups = {row['pgid'] for row in owner.refresh()
              if owner.matches(row) and owner.owned[row['pid']][1] is launch}
    for group in groups:
        valid_id(group)
        owner.group(launch, sig, group)


def load_record(path):
    value = json.loads(path.read_text())
    valid_id(value['pid'])
    if not isinstance(value.get('start'), str) or not value['start'].isdigit():
        raise ValueError('invalid runtime launch generation')
    return value


def stop(path):
    if not path.exists():
        return 0
    value = load_record(path)
    owner = OwnedProcesses()
    row = next((row for row in owner.snapshot() if row['pid'] == value['pid']), None)
    if row is None:
        path.unlink()
        return 0
    launch = owner.record(value['pid'], expected_start=value['start'], watch=True)
    try:
        settle(owner, launch, signal.SIGTERM)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            owner.refresh()
            if not any(root is launch for _, root in owner.owned.values()):
                return 0
            time.sleep(.1)
        settle(owner, launch, signal.SIGKILL)
        return 0
    finally:
        owner.close()


def run(path, command):
    if not command:
        raise ValueError('runtime command is missing')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
    owner = OwnedProcesses()
    child = None
    launch = None
    try:
        child = subprocess.Popen(command, start_new_session=True)
        launch = owner.record(child.pid, fresh_launch=True, watch=True)
        with os.fdopen(fd, 'w') as output:
            fd = None
            json.dump({'pid': launch.pid, 'start': launch.start}, output)
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(sig, lambda _sig, _frame: settle(owner, launch, signal.SIGTERM))
        return child.wait()
    finally:
        if fd is not None:
            os.close(fd)
        if launch is not None:
            settle(owner, launch, signal.SIGTERM)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                owner.refresh()
                if not any(root is launch for _, root in owner.owned.values()):
                    break
                time.sleep(.1)
            settle(owner, launch, signal.SIGKILL)
        owner.close()
        if path.exists() and (launch is None or load_record(path) == {'pid': launch.pid, 'start': launch.start}):
            path.unlink()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['run', 'stop'])
    parser.add_argument('--record', type=Path, required=True)
    argv = sys.argv[1:]
    split = argv.index('--') if '--' in argv else len(argv)
    args = parser.parse_args(argv[:split])
    command = argv[split + 1:]
    try:
        result = stop(args.record) if args.mode == 'stop' else run(args.record, command)
        sys.exit(result if result >= 0 else 128 - result)
    except (OSError, ValueError, KeyError):
        print('MP-11 runtime ownership guard refused an invalid or stale launch', file=sys.stderr)
        sys.exit(2)

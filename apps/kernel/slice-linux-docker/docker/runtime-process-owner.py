#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: supervise one runtime launch and its owned descendants."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import tempfile

sys.path[:0] = [str(Path(__file__).resolve().parent), str(Path(__file__).resolve().parent.parent)]
from owned_process_signals import OwnedProcesses, valid_id


def settle(owner, launch, sig):
    groups = {row['pgid'] for row in owner.refresh()
              if owner.matches(row) and owner.owned[row['pid']][1] is launch}
    for group in groups:
        valid_id(group)
        owner.group(launch, sig, group)


def load_record(path):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW)) as source:
        value = json.load(source)
    valid_id(value['pid'])
    if not isinstance(value.get('start'), str) or not value['start'].isdigit():
        raise ValueError('invalid runtime launch generation')
    return value


def remove_stale_record(path):
    try:
        identity = path.stat(follow_symlinks=False)
    except FileNotFoundError:
        return None
    try:
        value = load_record(path)
        row = next((row for row in OwnedProcesses().snapshot() if row['pid'] == value['pid']), None)
        if row is not None and row['start'] == value['start'] and row.get('state') != 'Z':
            return value
        # MP-08 / MP-11: an unclean stop leaves only a receipt, not authority over
        # a reused PID. Remove that exact stale receipt, never signal its successor.
        current = path.stat(follow_symlinks=False)
        if (identity.st_dev, identity.st_ino) != (current.st_dev, current.st_ino) or load_record(path) != value:
            raise ValueError('runtime launch record changed during stale cleanup')
        path.unlink()
        return None
    except FileNotFoundError:
        return None


def stop(path):
    value = remove_stale_record(path)
    if value is None:
        return 0
    owner = OwnedProcesses()
    try:
        launch = owner.record(value['pid'], expected_start=value['start'], watch=True)
    except ValueError:
        # The process can exit or be reused between the two metadata reads.
        if remove_stale_record(path) is None:
            return 0
        raise
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
    # The advisory lock is released by the OS even after SIGKILL. It prevents
    # concurrent publishers from launching a losing command before os.link.
    lock_fd = os.open(str(path) + '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if remove_stale_record(path) is not None:
            raise FileExistsError('runtime launch is still live')
    except BaseException:
        os.close(lock_fd)
        raise
    owner = OwnedProcesses()
    launch = None
    published = None
    temporary = None
    try:
        child = subprocess.Popen(command, start_new_session=True)
        launch = owner.record(child.pid, fresh_launch=True, watch=True)
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(sig, lambda _sig, _frame: settle(owner, launch, signal.SIGTERM))
        fd, temporary = tempfile.mkstemp(prefix='.runtime-owner-', dir=path.parent)
        with os.fdopen(fd, 'w') as output:
            json.dump({'pid': launch.pid, 'start': launch.start}, output)
            output.flush()
            os.fsync(output.fileno())
        # Publish a complete receipt atomically without replacing another launch.
        os.link(temporary, path, follow_symlinks=False)
        published = path.stat(follow_symlinks=False)
        os.unlink(temporary)
        temporary = None
        return child.wait()
    finally:
        if temporary is not None:
            os.unlink(temporary)
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
        if published is not None:
            try:
                current = path.stat(follow_symlinks=False)
                if (published.st_dev, published.st_ino) == (current.st_dev, current.st_ino):
                    path.unlink()
            except FileNotFoundError:
                pass
        os.close(lock_fd)


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

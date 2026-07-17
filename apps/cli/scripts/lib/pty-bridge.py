#!/usr/bin/env python3
"""Run a command in a real PTY while bridging parent stdin/stdout byte-for-byte."""

from __future__ import annotations

import argparse
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--columns", type=int, required=True)
    parser.add_argument("--rows", type=int, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    parsed = parser.parse_args()
    if parsed.command and parsed.command[0] == "--":
        parsed.command = parsed.command[1:]
    if not parsed.command:
        parser.error("command is required after --")
    return parsed


def write_all(fd: int, data: bytes) -> None:
    while data:
        written = os.write(fd, data)
        data = data[written:]


def main() -> int:
    args = parse_args()
    child_pid, master_fd = pty.fork()
    if child_pid == 0:
        os.execvpe(args.command[0], args.command, os.environ)

    current_columns = args.columns

    def set_window_size(columns: int) -> None:
        fcntl.ioctl(
            master_fd,
            termios.TIOCSWINSZ,
            struct.pack("HHHH", args.rows, columns, 0, 0),
        )
        os.kill(child_pid, signal.SIGWINCH)

    def toggle_window_width(_signal_number: int, _frame: object) -> None:
        nonlocal current_columns
        current_columns = args.columns - 1 if current_columns == args.columns else args.columns
        set_window_size(current_columns)

    signal.signal(signal.SIGUSR1, toggle_window_width)
    set_window_size(current_columns)
    stdin_open = True
    try:
        while True:
            readers = [master_fd]
            if stdin_open:
                readers.append(sys.stdin.fileno())
            ready, _, _ = select.select(readers, [], [])
            if master_fd in ready:
                try:
                    data = os.read(master_fd, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not data:
                    break
                write_all(sys.stdout.fileno(), data)
            if stdin_open and sys.stdin.fileno() in ready:
                data = os.read(sys.stdin.fileno(), 65536)
                if data:
                    write_all(master_fd, data)
                else:
                    stdin_open = False
    finally:
        os.close(master_fd)

    _, status = os.waitpid(child_pid, 0)
    return os.waitstatus_to_exitcode(status)


if __name__ == "__main__":
    raise SystemExit(main())

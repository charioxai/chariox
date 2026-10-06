"""MP-11: shared PID/start-time ownership guard for Python group signals.

Only public PID/parent/group/start metadata is inspected. Unknown or reused
members fail closed, including after the group's original leader has exited.
"""
import os
from pathlib import Path
import subprocess
import sys
import threading
import time


def valid_id(pid):
    if type(pid) is not int or pid <= 1:
        raise ValueError("invalid owned process identity")
    return pid


def process_snapshot():
    if sys.platform == "linux":
        rows = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                text = (entry / "stat").read_text()
                fields = text[text.rindex(")") + 2:].split()
                rows.append(dict(pid=int(entry.name), ppid=int(fields[1]),
                                 pgid=int(fields[2]), sid=int(fields[3]), start=fields[19]))
            except (FileNotFoundError, ProcessLookupError):
                pass
        return rows
    if sys.platform != "darwin":
        raise ValueError("owned group inspection is unsupported on this platform")
    text = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,pgid=,sess=,lstart="],
                                   text=True, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
    rows = []
    for line in text.splitlines():
        pid, ppid, pgid, session, start = line.split(maxsplit=4)
        rows.append(dict(pid=int(pid), ppid=int(ppid), pgid=int(pgid), session=session, start=start))
    return rows


class OwnedProcesses:
    def __init__(self, snapshot=process_snapshot, send_group=os.killpg, send_pid=os.kill):
        self.snapshot = snapshot
        self.send_group = send_group
        self.send_pid = send_pid
        self.owned = {}
        self.roots = set()
        self.sessions = {}
        self.lock = threading.RLock()
        self.closed = threading.Event()
        self.thread = None

    def matches(self, row):
        return bool(row and row.get("start") and self.owned.get(row["pid"]) == row["start"])

    def refresh(self, rows=None):
        with self.lock:
            rows = self.snapshot() if rows is None else rows
            changed = True
            while changed:
                changed = False
                for row in rows:
                    if row["pid"] in self.owned or row["pid"] <= 1 or not row.get("start"):
                        continue
                    # A live recorded setsid leader pins its run's exclusive
                    # session, including children reparented before a sample.
                    in_session = any(root["pid"] in self.roots and root.get("sid") == root["pid"]
                                     and root.get("sid") == row.get("sid") and self.matches(root) for root in rows)
                    in_session = in_session or any(root["pid"] in self.sessions
                        and self.sessions[root["pid"]] == row.get("session")
                        and root.get("session") == row.get("session") and self.matches(root) for root in rows)
                    if in_session or any(parent["pid"] == row["ppid"] and self.matches(parent) for parent in rows):
                        self.owned[row["pid"]] = row["start"]
                        changed = True
            return rows

    def record(self, pid, watch=False):
        valid_id(pid)
        with self.lock:
            rows = self.snapshot()
            row = next((row for row in rows if row["pid"] == pid), None)
            if not row or not row.get("start"):
                raise ValueError("owned process start identity unavailable")
            if pid in self.owned and not self.matches(row):
                raise ValueError("unowned reused process identity")
            self.owned[pid] = row["start"]
            self.roots.add(pid)
            if row.get("session") and row["pgid"] == pid and os.getsid(pid) == pid:
                self.sessions[pid] = row["session"]
            self.refresh(rows)
        if watch and self.thread is None:
            self.thread = threading.Thread(target=self._watch, daemon=True)
            self.thread.start()
        return pid

    def _watch(self):
        while not self.closed.wait(0.025):
            try:
                self.refresh()
            except (OSError, ValueError):
                pass  # The immediate signal-time read still fails closed.

    def close(self):
        self.closed.set()
        if self.thread:
            self.thread.join(timeout=1)

    def group(self, pgid, sig):
        valid_id(pgid)
        with self.lock:
            rows = self.refresh()
            members = [row for row in rows if row["pgid"] == pgid]
            if not members:
                return False
            if pgid not in self.roots and not any(row["pid"] == pgid and self.matches(row) for row in members):
                raise ValueError("unowned process group")
            if any(not self.matches(row) for row in members):
                raise ValueError("unowned process group member")
            self.send_group(pgid, sig)
            return True

    def pid(self, pid, sig):
        valid_id(pid)
        with self.lock:
            row = next((row for row in self.refresh() if row["pid"] == pid), None)
            if not row:
                return False
            if not self.matches(row):
                raise ValueError("unowned process identity")
            self.send_pid(pid, sig)
            return True

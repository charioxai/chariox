"""MP-11: shared PID/start-time ownership guard for Python group signals.

Only public PID/parent/group/start metadata is inspected. Unknown or reused
members fail closed, including after the group's original leader has exited.
"""
import os
from pathlib import Path
import subprocess
import sys
import threading
import weakref
from dataclasses import dataclass


@dataclass(frozen=True, eq=False)
class _Launch:
    pid: int
    start: str
    session: str = None


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
        self.roots = {}
        self.handles = weakref.WeakSet()
        self.lock = threading.RLock()
        self.closed = threading.Event()
        self.thread = None

    def matches(self, row):
        return bool(row and row.get("start") and self.owned.get(row["pid"], (None, None))[0] == row["start"])

    def refresh(self, rows=None):
        with self.lock:
            rows = self.snapshot() if rows is None else rows
            live = {row["pid"]: row for row in rows}
            for pid, (start, launch) in list(self.owned.items()):
                if live.get(pid, {}).get("start") != start:
                    del self.owned[pid]
            for pid, launch in list(self.roots.items()):
                if live.get(pid, {}).get("start") != launch.start and not any(
                        owner is launch for _, owner in self.owned.values()):
                    del self.roots[pid]
            changed = True
            while changed:
                changed = False
                for row in rows:
                    if row["pid"] in self.owned or row["pid"] <= 1 or not row.get("start"):
                        continue
                    parent = live.get(row["ppid"])
                    launch = self.owned.get(row["ppid"], (None, None))[1] if self.matches(parent) else None
                    if launch is None:
                        for root in self.roots.values():
                            leader = live.get(root.pid)
                            if not self.matches(leader) or leader["start"] != root.start:
                                continue
                            if (leader.get("sid") == root.pid and row.get("sid") == root.pid
                                    or root.session and row.get("session") == root.session):
                                launch = root
                                break
                    if launch is not None:
                        self.owned[row["pid"]] = (row["start"], launch)
                        changed = True
            return rows

    def record(self, pid, watch=False, fresh_launch=False, expected_start=None):
        valid_id(pid)
        with self.lock:
            rows = self.snapshot()
            row = next((row for row in rows if row["pid"] == pid), None)
            if not row or not row.get("start"):
                raise ValueError("owned process start identity unavailable")
            if expected_start is not None and row["start"] != expected_start:
                raise ValueError("unowned reused process identity")
            previous = self.roots.get(pid)
            if previous is not None and previous.start != row["start"] and not fresh_launch:
                raise ValueError("unowned reused process identity")
            self.refresh(rows)
            launch = previous if previous is not None and previous.start == row["start"] else _Launch(
                pid, row["start"], row.get("session") if row.get("session")
                and row["pgid"] == pid and os.getsid(pid) == pid else None)
            self.handles.add(launch)
            self.roots[pid] = launch
            self.owned[pid] = (row["start"], launch)
            self.refresh(rows)
        if watch and self.thread is None:
            self.thread = threading.Thread(target=self._watch, daemon=True)
            self.thread.start()
        return launch

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

    def target(self, launch):
        if not isinstance(launch, _Launch):
            valid_id(launch)
            raise ValueError("owned process generation handle required")
        valid_id(launch.pid)
        if launch not in self.handles:
            raise ValueError("unowned process generation handle")
        return launch

    def group(self, launch, sig, pgid=None):
        with self.lock:
            launch = self.target(launch)
            pgid = valid_id(launch.pid if pgid is None else pgid)
            members = [row for row in self.refresh() if row["pgid"] == pgid]
            if not members:
                return False
            if any(not self.matches(row) or self.owned[row["pid"]][1] is not launch for row in members):
                raise ValueError("unowned process group member")
            self.send_group(pgid, sig)
            return True

    def pid(self, launch, sig):
        with self.lock:
            launch = self.target(launch)
            row = next((row for row in self.refresh() if row["pid"] == launch.pid), None)
            if not row:
                return False
            if not self.matches(row) or row["start"] != launch.start or self.owned[row["pid"]][1] is not launch:
                raise ValueError("unowned process identity")
            self.send_pid(launch.pid, sig)
            return True

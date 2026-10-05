#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: signal only an identified fixture-owned process tree."""
import json
import os
from pathlib import Path
import sys


def process_stat(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return {"parent": int(fields[1]), "group": int(fields[2]), "start": fields[19]}


def signal_owned_group(identity, signum, read_stat=process_stat, pids=None, send=os.killpg):
    pid = identity["pid"]
    if type(pid) is not int or pid <= 1:
        raise ValueError("unsafe process group")
    leader = read_stat(pid)
    if leader["group"] != pid or leader["start"] != identity["start"]:
        raise ValueError("stale process group")
    if pids is None:
        pids = [int(p.name) for p in Path("/proc").iterdir() if p.name.isdigit()]
    members = {}
    for member in pids:
        try:
            entry = read_stat(member)
            if entry["group"] == pid:
                members[member] = entry
        except (OSError, ValueError):
            continue
    if pid not in members:
        raise ValueError("missing group leader")
    for member in members:
        cursor, seen = member, set()
        while cursor != pid:
            if cursor <= 1 or cursor in seen or cursor not in members:
                raise ValueError("foreign group member")
            seen.add(cursor)
            cursor = members[cursor]["parent"]
    if read_stat(pid) != leader:
        raise ValueError("changed group leader")
    send(pid, signum)


def signal_owned_child(child, parent, signum, read_stat=process_stat, send=os.kill):
    pid, parent_pid = child["pid"], parent["pid"]
    if type(pid) is not int or type(parent_pid) is not int or min(pid, parent_pid) <= 1:
        raise ValueError("unsafe child PID")
    if read_stat(parent_pid)["start"] != parent["start"]:
        raise ValueError("stale parent")
    actual = read_stat(pid)
    if actual["start"] != child["start"] or actual["parent"] != parent_pid:
        raise ValueError("foreign or stale child")
    send(pid, signum)


if __name__ == "__main__":
    if sys.argv[1] == "start":
        pid = os.getpid()
        if pid <= 1:
            raise ValueError("unsafe fixture PID")
        if os.getpgrp() != pid:
            os.setsid()
        Path(sys.argv[2]).write_text(json.dumps({"pid": pid, "start": process_stat(pid)["start"]}))
        os.execv("/bin/bash", ["bash", *sys.argv[3:]])
    elif sys.argv[1] == "signal":
        identity = json.loads(Path(sys.argv[2]).read_text())
        signal_owned_group(identity, int(sys.argv[3]))
    else:
        raise ValueError("unknown fixture process operation")

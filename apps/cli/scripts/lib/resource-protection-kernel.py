#!/usr/bin/env python3
"""Capture the identity of this drill's independently started kernel."""
import json
import os
import sys
from pathlib import Path

leaf, binary, identity = sys.argv[1:]
pid = os.getpid()
if os.getpgrp() != pid:
    os.setpgid(0, 0)
Path(leaf, "cgroup.procs").write_text(str(pid))
stat = Path("/proc/self/stat").read_text().rsplit(")", 1)[1].split()
Path(identity).write_text(json.dumps({
    "pid": pid,
    "processGroupId": os.getpgrp(),
    "startedAtTicks": stat[19],
}))
os.execv(binary, [binary])

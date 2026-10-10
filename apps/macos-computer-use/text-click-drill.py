#!/usr/bin/env python3
"""Unattended GUI fixture drill. No permission prompts or direct AX access."""
import os
from pathlib import Path
import re
import signal
import subprocess
import time

build = Path("/Users/miguel/.chariox/dev/cumac/build")
evidence = Path.home() / ".codex/evidence/browser-computer-use-macos" / time.strftime("ax-text-click-%Y%m%d-%H%M%S")
evidence.mkdir(parents=True, exist_ok=False)
fixture = build / "Chariox Computer Fixture.app"
helper = build / "Chariox Computer Helper.app"
pid = None


def read(name):
    path = evidence / name
    return path.read_text() if path.exists() else ""


try:
    subprocess.run(["open", "-n", "--stdout", str(evidence / "fixture.out"), "--stderr",
                    str(evidence / "fixture.err"), str(fixture), "--args", "--text-drill"], check=True)
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        output = read("fixture.out")
        identity = re.search(r"fixturePID=(\d+) windowID=(\d+)", output)
        point = re.search(r"clickX=([\d.-]+) clickY=([\d.-]+) expectedLocation=(\d+)", output)
        if identity:
            pid = int(identity[1])
        if identity and point:
            break
        time.sleep(0.1)
    else:
        raise RuntimeError("fixture did not publish its identity/line-5 point")
    subprocess.run(["open", "-n", "-g", "-W", "--stdout", str(evidence / "helper.out"),
                    "--stderr", str(evidence / "helper.err"), str(helper), "--args", "--enable-fixture",
                    "--fixture-pid", str(pid), "--window-id", identity[2], "--target", "text",
                    "--click-at", point[1], point[2], "--click"], check=True, timeout=8)
    receipt = read("helper.out") + read("helper.err")
    print(receipt, end="")
    if "refused: permission" in receipt:
        verdict = "BLOCKED: helper public result refused: permission. Owner recheck must prove TextEdit AX caret cell."
        code = 2
    else:
        expected = point[3]
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline and f"fixture selection location={expected} length=0" not in read("fixture.out"):
            time.sleep(0.05)
        helper_pass = f"path=AXSelectedTextRange; observed AXSelectedTextRange location={expected} length=0;" in receipt
        fixture_pass = f"fixture selection location={expected} length=0" in read("fixture.out")
        code = 0 if helper_pass and fixture_pass else 1
        verdict = "PASS: helper readback and NSTextView selection agree at line 5 mid-word." if code == 0 else "FAIL: caret movement not proven; no replay."
    (evidence / "RESULT.txt").write_text(verdict + "\n")
    print(verdict)
    print(f"Evidence: {evidence}")
finally:
    if pid:
        # Only the process started by this drill, never an owner's older fixture.
        command = subprocess.run(["ps", "-p", str(pid), "-o", "command="], capture_output=True, text=True).stdout.strip()
        if command == str(fixture / "Contents/MacOS/fixture") + " --text-drill":
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        # The app's 20-second timer also closes it if this runner is interrupted.
raise SystemExit(code)

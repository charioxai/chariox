#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11: real outside Codex -> compiled CLI -> kernel -> TUI.

Refuses the grant through the TUI; never approves or changes a provider login.
Requires pyte/Pillow, real built binaries, and an approved product-linked profile.
Exit 2 means a real provider/resource blocked the drill, never acceptance.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import shutil
import socket
import struct
import subprocess
import tempfile
import termios
import time
import tomllib

import pyte
from PIL import Image, ImageDraw, ImageFont

from lib.kernel_access_requester_outcome import verify_requester_exit

MP = ["MP-08", "MP-10", "MP-11"]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--kernel", required=True)
parser.add_argument("--cli", required=True)
parser.add_argument("--codex-profile", required=True)
parser.add_argument("--output", required=True)
parser.add_argument("--source", required=True)
parser.add_argument("--local-cli", action="store_true", help="Supplementary regression only; no provider acceptance")
args = parser.parse_args()
out = Path(args.output).resolve()
out.mkdir(parents=True, exist_ok=False)
root = Path.home() / ".chariox/dev/kaext-structured"
root.mkdir(parents=True, exist_ok=True)
state = Path(tempfile.mkdtemp(prefix="live-", dir=root))
work = Path(tempfile.mkdtemp(prefix="agent-kaext-requester-", dir="/root/work"))
children = []
master = None
raw = bytearray()
screen = pyte.Screen(160, 50)
# Never reflect terminal device requests into our stdout.
pyte.Screen.report_device_status = lambda *a, **k: None
stream = pyte.Stream(screen)
result = {"mpItems": MP, "source": args.source, "provider": "codex", "supplementary": args.local_cli,
          "startedAt": time.time(), "status": "FAIL", "approvalsSent": 0}
for key in ("kernel", "cli"):
    binary = Path(getattr(args, key)).resolve()
    result[key] = {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}


def emit(**value):
    print(json.dumps({"mpItems": MP, **value}), flush=True)


def stop(child):
    if not isinstance(child.pid, int) or child.pid <= 1:
        raise RuntimeError("MP-11 unsafe process ID")
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=10)


def start(command, env, name):
    with open(state / (name + ".stdout"), "w") as stdout, open(state / (name + ".stderr"), "w") as stderr:
        child = subprocess.Popen(command, cwd=work, env=env, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
    children.append(child)
    return child


def sample():
    memory = next(int(x.split()[1]) * 1024 for x in Path("/proc/meminfo").read_text().splitlines() if x.startswith("MemAvailable:"))
    disk = shutil.disk_usage("/").free
    with open(out / "resources.jsonl", "a") as f:
        f.write(json.dumps({"mpItems": MP, "at": time.time(), "memAvailableBytes": memory, "diskFreeBytes": disk}) + "\n")
    if memory < 9 * 2**30 or disk < 10 * 2**30:
        raise RuntimeError("MP-11 resource floor")


def pump(seconds=.3):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if master is not None and select.select([master], [], [], .05)[0]:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            raw.extend(data)
            stream.feed(data.decode("utf8", "replace"))


def frame():
    return "\n".join(screen.display)


def capture(name):
    pump(.5)
    (out / (name + ".txt")).write_text(frame())
    font = ImageFont.truetype("/usr/share/fonts/opentype/urw-base35/NimbusMonoPS-Regular.otf", 16)
    image = Image.new("RGB", (1620, 1020), (18, 18, 22))
    draw = ImageDraw.Draw(image)
    for y, line in enumerate(screen.display):
        draw.text((10, 10 + y * 20), line, font=font, fill=(222, 222, 230))
    image.save(out / (name + ".png"))


def await_text(text, timeout=35):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        pump(.1)
        if text in frame():
            return
    raise RuntimeError("MP-10 TUI missing " + text)


try:
    sample()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("CHARIOX_", "CODEX_", "CLAUDE_", "OPENCODE_", "XDG_"))}
    env.update(CHARIOX_HOME=str(state / "kernel"), CHARIOX_LOG_DIR=str(state / "logs"),
               CHARIOX_DAEMON_SOCKET=str(state / "kernel.sock"), HOME=str(state / "user"),
               CODEX_HOME=str(state / "empty-codex"), CLAUDE_CONFIG_DIR=str(state / "empty-claude"),
               OPENCODE_CONFIG_DIR=str(state / "empty-opencode"), XDG_CONFIG_HOME=str(state / "config"),
               XDG_DATA_HOME=str(state / "data"), XDG_STATE_HOME=str(state / "xdg-state"),
               XDG_CACHE_HOME=str(state / "cache"), TERM="xterm-256color", TMPDIR=str(state / "tmp"))
    Path(env["TMPDIR"]).mkdir()
    Path(env["HOME"]).mkdir()
    for key in ("CHARIOX_KERNEL_PORT", "CHARIOX_MCP_PORT", "CHARIOX_CODEX_PORT", "CHARIOX_OPENCODE_PORT"):
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            env[key] = str(probe.getsockname()[1])
    kernel = start([args.kernel], env, "kernel")
    end = time.monotonic() + 35
    while not (state / "kernel.sock").exists():
        if kernel.poll() is not None or time.monotonic() > end:
            raise RuntimeError("MP-10 kernel unavailable")
        time.sleep(.1)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))
    tui = subprocess.Popen([args.cli, "--kernel-url", "ws://127.0.0.1:" + env["CHARIOX_KERNEL_PORT"]],
                           cwd=work, env=env, stdin=slave, stdout=slave, stderr=slave)
    children.append(tui)
    os.close(slave)
    await_text("No session attached")
    capture("01-real-tui")
    outside = env.copy()
    # This is the linked product profile, never a credential copy or default agent home.
    outside["CODEX_HOME"] = str(Path(args.codex_profile).resolve())
    helper = state / "request.mjs"
    helper.write_text('''import { readFileSync, readlinkSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
let pid = process.ppid, holder;
while (Number.isSafeInteger(pid) && pid > 1) {
  const fields = readFileSync(`/proc/${pid}/stat`, 'utf8').split(')').at(-1).trim().split(/\\s+/);
  const executable = readlinkSync(`/proc/${pid}/exe`);
  if (executable.endsWith('/codex')) { holder = {pid, executable, process_start_id: fields[19]}; break; }
  pid = Number(fields[1]);
}
if (!holder) throw new Error('MP-11 no real outside Codex ancestor');
writeFileSync(process.argv[4], JSON.stringify(holder));
const result = spawnSync(process.argv[2], ['access', 'request', '--socket', process.argv[3], '--holder-pid', String(holder.pid), '--minutes', '6'], {stdio:'inherit'});
process.exitCode = result.status ?? 1;
''')
    if args.local_cli:
        # Own CLI peer is a real process; this mode never claims provider acceptance.
        requester = start([args.cli, "access", "request", "--socket", str(state / "kernel.sock"), "--holder-pid", str(os.getpid()), "--minutes", "6"], env, "outside")
    else:
        bun = shutil.which("bun")
        if bun is None:
            raise RuntimeError("MP-10 Bun unavailable for the outside Codex helper")
        import shlex
        command = " ".join(shlex.quote(x) for x in [bun, str(helper), args.cli, str(state / "kernel.sock"), str(state / "requester.json")])
        prompt = "MP-08 / MP-10 / MP-11 real outside Codex requester drill. Run exactly " + command + ". Poll until it completes. The human will REFUSE the grant in the real TUI; this expected refusal is successful validation. Use only that command. Do not read credentials/config/token/vault files, approve anything, signal processes, or change logins. Exit after the refusal."
        command = ["codex", "exec", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check", "--json", "-m", "gpt-6.1-sol", "-c", 'model_reasoning_effort="low"', "-C", str(work), prompt]
        # Read only non-secret MCP server names; disable them for this invocation.
        config = Path(outside["CODEX_HOME"]) / "config.toml"
        servers = tomllib.loads(config.read_text()).get("mcp_servers", {}) if config.exists() else {}
        import re
        for name in servers:
            if not re.fullmatch(r"[A-Za-z0-9_-]+", name):
                raise RuntimeError("MP-11 invalid MCP name")
            command[1:1] = ["-c", "mcp_servers." + name + ".enabled=false"]
        requester = start(command, outside, "outside")
    end = time.monotonic() + 120
    while "passkey request" not in frame() and "Grant external agent access" not in frame():
        pump(.2)
        sample()
        if requester.poll() is not None:
            diagnostics = (state / "outside.stdout").read_text() + (state / "outside.stderr").read_text()
            if "401" in diagnostics or "Unauthorized" in diagnostics:
                result.update(status="BLOCKED", firstFailingSeam="Official Codex account authentication: HTTP 401 before grant request", ownerAction="Identify a working approved product-linked Codex profile; no shared-login changes authorized")
                emit(status="BLOCKED", firstFailingSeam=result["firstFailingSeam"])
                raise SystemExit(2)
            raise RuntimeError("MP-10 outside requester exited before grant")
        if time.monotonic() > end:
            raise RuntimeError("MP-10 outside requester did not request grant")
    os.write(master, b"\x1b[19~")
    await_text("Grant external agent access")
    capture("02-structured-requester-popup")
    labels = frame()
    if not args.local_cli:
        result["requester"] = json.loads((state / "requester.json").read_text())
    if "Requester:" not in labels or "Executable:" not in labels or "Process start:" not in labels:
        raise RuntimeError("MP-11 structured requester label absent")
    if not args.local_cli:
        identity = result["requester"]
        if "Requester: Codex" not in labels or "PID " + str(identity["pid"]) not in labels or identity["process_start_id"] not in labels:
            raise RuntimeError("MP-11 TUI requester differs from real Codex OS identity")
        result["requester"] = identity
    # Ctrl+R is the real refusal hotkey. No passkey is typed, and no login changes.
    os.write(master, b"\x12")
    pump(1)
    capture("03-refused-popup")
    if "Grant external agent access" in frame():
        raise RuntimeError("MP-11 refused popup stayed open")
    requester.wait(timeout=45)
    diagnostics = (state / "outside.stderr").read_text() if args.local_cli else ""
    refusal = verify_requester_exit(requester.returncode, diagnostics, local_cli=args.local_cli)
    if refusal is not None:
        result["requesterDiagnostic"] = refusal
    result.update(status="PASS", requesterExit=requester.returncode,
                  established="Real TUI structured OS requester projection and refusal" if not args.local_cli else "Supplementary real local CLI/TUI requester projection and refusal")
except SystemExit:
    raise
except Exception as error:
    capture("failure")
    result["failure"] = str(error)
    emit(status="FAIL", failure=str(error))
    raise
finally:
    # Public execution timeline only; provider text/config/credentials never copied.
    events = []
    provider_output = state / "outside.stdout"
    if not args.local_cli and provider_output.exists():
        for line in provider_output.read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            item = event.get("item", {})
            public = {"type": event.get("type"), "item_type": item.get("type"),
                      "status": item.get("status"), "exit_code": item.get("exit_code")}
            if item.get("type") == "command_execution" and str(state / "request.mjs") in item.get("command", ""):
                public["command"] = item["command"]
            events.append(public)
        (out / "provider-events.json").write_text(json.dumps({"mpItems": MP, "events": events}, indent=2) + "\n")
    for child in reversed(children):
        stop(child)
    if master is not None:
        os.close(master)
    # Stop only descendants tagged with this exact lane-owned state; inspect only
    # public HOME/state paths, never credentials or arbitrary process environment.
    remaining = []
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit() or int(proc.name) <= 1:
            continue
        try:
            for item in (proc / "environ").read_bytes().split(b"\0"):
                key, _, value = item.partition(b"=")
                if key in (b"HOME", b"CHARIOX_HOME", b"TMPDIR") and value.decode("utf8", "replace").startswith(str(state) + "/"):
                    remaining.append(int(proc.name))
                    break
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    if remaining:
        result["cleanup"] = {"ownedRemainingPids": remaining, "statePreserved": str(state)}
        result["status"] = "FAIL"
    else:
        shutil.rmtree(state)
        shutil.rmtree(work)
        result["cleanup"] = {"ownedRemainingPids": [], "stateRemoved": True, "workspaceRemoved": True}
    (out / "terminal.ansi").write_bytes(raw)
    result["finishedAt"] = time.time()
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    sample()
    emit(status=result["status"], cleanup=result["cleanup"])
    if result["status"] == "FAIL":
        raise SystemExit(1)

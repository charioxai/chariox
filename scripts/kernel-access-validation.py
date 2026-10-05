#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: Linux acceptance fixtures, private state, public receipts.

Uses only existing cfg(test) passkey fixtures. No live vault/provider account,
Cloud, relay machine, Docker resource or owner authentication is accessed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time

MP = ["MP-08", "MP-10", "MP-11"]
LOCK = "/root/.chariox/dev/browser-resume-20260930/locks/rust-compile.lock"
ROOT = Path(__file__).resolve().parents[1]


class ValidationCancelled(Exception):
    def __init__(self, signum):
        self.signum = signum
        super().__init__(f"MP-11: validation cancelled by signal {signum}")


def source_identity(expected=None):
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()
    # Only these operator handoffs are outside the source under validation.
    dirty = git("status", "--porcelain", "--untracked-files=all", "--", ".",
                ":!LANE_STATUS.md", ":!PUSH_READY.md")
    current = (git("rev-parse", "HEAD"), git("rev-parse", "HEAD^{tree}"))
    if dirty or (expected and current != expected):
        raise RuntimeError("MP-11: source checkout changed; require clean committed source")
    return current


def identity(pid):
    if not isinstance(pid, int) or pid <= 1:
        return None
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return int(fields[1]), int(fields[19]), fields[0]
    except (OSError, ValueError, IndexError):
        return None


def collect_owned(owned):
    rows = {}
    for path in Path("/proc").iterdir():
        if path.name.isdigit():
            row = identity(int(path.name))
            if row:
                rows[int(path.name)] = row
    changed = True
    while changed:
        changed = False
        for pid, (parent, birth, _) in rows.items():
            parent_row = rows.get(parent)
            if pid not in owned and parent_row and owned.get(parent) == parent_row[1]:
                owned[pid] = birth
                changed = True


def stop_owned(owned):
    # No process group signals: exact observed birth identities and pidfds only.
    for sig in (signal.SIGTERM, signal.SIGKILL):
        collect_owned(owned)
        for pid, birth in reversed(list(owned.items())):
            if not isinstance(pid, int) or pid <= 1:
                raise RuntimeError("MP-11: invalid cleanup PID")
            current = identity(pid)
            if not current or current[1] != birth or current[2] == "Z":
                continue
            try:
                fd = os.pidfd_open(pid)
                try:
                    after = identity(pid)
                    if after and after[1] == birth:
                        signal.pidfd_send_signal(fd, sig)
                finally:
                    os.close(fd)
            except ProcessLookupError:
                pass
        time.sleep(0.2)


def sample():
    fields = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    return {"at": time.time(), "available_gib": int(fields["MemAvailable"].split()[0]) / 1048576,
            "disk_free_gib": shutil.disk_usage("/").free / 1073741824}


def append(path, value):
    with path.open("a") as out:
        out.write(json.dumps(value) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--kit-only", action="store_true", help="validate newly changed kit against an already source-bound artifact")
    parser.add_argument("--artifact-receipt", type=Path)
    args = parser.parse_args()
    if os.uname().sysname != "Linux":
        parser.error("MP-11: run on reserved Linux builder; use SUDO_KIT.md on Mac")
    output = args.output.resolve()
    if output == ROOT or ROOT in output.parents or output.exists():
        parser.error("MP-11: output must be a new evidence directory outside checkout")
    output.mkdir(parents=True, mode=0o700)
    head, tree = source_identity()
    state = Path(tempfile.mkdtemp(prefix="kv-"))
    environment = os.environ.copy()
    for key in list(environment):
        if key.startswith(("CHARIOX_", "CODEX_", "CLAUDE_", "OPENCODE_", "XDG_")):
            environment.pop(key)
    environment.update(CHARIOX_HOME=str(state / "state"), HOME=str(state / "home"),
                       TMPDIR=str(state / "tmp"), CHARIOX_LOG_DIR=str(state / "logs"),
                       CARGO_HOME="/var/lib/chariox/dev/toolchains/cargo",
                       RUSTUP_HOME="/var/lib/chariox/dev/toolchains/rustup",
                       CARGO_TARGET_DIR=str(ROOT / "target"), CARGO_BUILD_JOBS="4",
                       CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
                       CARGO_INCREMENTAL="0", RUST_MIN_STACK="16777216", RUST_TEST_THREADS="1")
    pinned = list(Path("/root/.local/share/pnpm/store/v11/links/@/pnpm/9.15.0").glob(
        "*/node_modules/pnpm/bin/pnpm.cjs"))
    if len(pinned) != 1:
        raise RuntimeError("MP-11: expected one cached pnpm 9.15.0 executable")
    (state / "bin").mkdir()
    (state / "bin/pnpm").symlink_to(pinned[0])
    environment["PATH"] = str(state / "bin") + ":/var/lib/chariox/dev/toolchains/cargo/bin:" + environment["PATH"]
    for name in ("state", "home", "tmp", "logs"):
        (state / name).mkdir(mode=0o700)
    all_owned = {}
    results = []
    cancelled_signal = None
    cancellation_deferred = False

    def cancel(signum, _frame):
        nonlocal cancelled_signal
        if cancelled_signal is None:
            cancelled_signal = signum
            if not cancellation_deferred:
                raise ValidationCancelled(signum)

    def run(name, command):
        nonlocal cancellation_deferred
        source_identity((head, tree))
        before = sample()
        append(output / "resources.jsonl", before)
        if before["available_gib"] < 16 or before["disk_free_gib"] < 10:
            raise RuntimeError("MP-10: resource floor before " + name)
        started = time.time()
        owned = {}
        failure = None
        child = None
        with (output / (name + ".log")).open("w") as log:
            try:
                # A pending cancellation must not escape between spawn and registration.
                cancellation_deferred = True
                try:
                    child = subprocess.Popen(command, cwd=ROOT, env=environment, stdout=log,
                                             stderr=subprocess.STDOUT, start_new_session=True)
                    row = identity(child.pid)
                    if row:
                        owned[child.pid] = row[1]
                finally:
                    cancellation_deferred = False
                if cancelled_signal:
                    raise ValidationCancelled(cancelled_signal)
                while child.poll() is None:
                    collect_owned(owned)
                    current = sample()
                    append(output / "resources.jsonl", {"stage": name, **current})
                    if current["available_gib"] < 16 or current["disk_free_gib"] < 10:
                        failure = "MP-10: measured resource floor during " + name
                        break
                    time.sleep(2)
                source_identity((head, tree))
            except (ValidationCancelled, RuntimeError) as error:
                failure = str(error)
            finally:
                cancellation_deferred = True
                all_owned.update(owned)
                stop_owned(owned)
                if child:
                    child.wait()
                all_owned.update(owned)
                cancellation_deferred = False
        if child is None:
            raise RuntimeError("MP-11: stage did not start " + name)
        result = {"mp": MP, "head": head, "name": name, "command": command,
                  "tree": tree, "started": started, "finished": time.time(),
                  "exit": child.returncode, "cancelled_signal": cancelled_signal}
        result["script_sha256"] = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                   for path in [ROOT / "scripts/kernel-access-validation.py",
                                                ROOT / "scripts/sudo-kit.mjs", ROOT / "scripts/sudo-kit-smoke.mjs",
                                                ROOT / "scripts/sudo-kit-receipts.py"]}
        if failure:
            result["failure"] = failure
            result["exit"] = child.returncode or 1
        if "--nocapture" in command:
            summaries = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", (output / (name + ".log")).read_text())
            result["rust_summaries"] = summaries
            if child.returncode == 0 and not any(int(passed) for passed, _, _ in summaries):
                result["exit"] = 1
                result["failure"] = "selector ran zero passing tests"
        results.append(result)
        append(output / "checks.jsonl", result)
        print(f"MP-08/MP-10/MP-11 {name}: exit {result['exit']}", flush=True)
        if cancelled_signal:
            raise ValidationCancelled(cancelled_signal)
        if result["exit"]:
            raise RuntimeError("MP-11: first failing seam " + name)

    previous_handlers = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        for sig in previous_handlers:
            signal.signal(sig, cancel)
        run("watchdog", ["python3", "scripts/kernel-access-validation.test.py"])
        if args.kit_only:
            run("popup-client", ["node", "--test", "--test-concurrency=1", "apps/cli/dist/passkey-popup-controller.test.js"])
        if args.kit_only:
            if not args.artifact_receipt:
                raise RuntimeError("MP-11: kit-only requires source/binary receipt")
            receipt = json.loads(args.artifact_receipt.read_text())
            artifact = Path(receipt["binary"])
            with artifact.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if (receipt["head"] != head or receipt.get("tree") != tree
                    or receipt.get("source_policy") != "clean_checkout" or digest != receipt["sha256"]):
                raise RuntimeError("MP-11: artifact/source identity mismatch")
            run("kit-smoke", ["flock", LOCK, "node", "scripts/sudo-kit-smoke.mjs", str(artifact)])
            run("sandboxed-pty-child", ["flock", LOCK, str(artifact), "sandboxed_pty_survives_launching_thread_exit",
                                        "--nocapture", "--test-threads=1", "--ignored"])
            source_identity((head, tree))
            return
        run("node-install", ["pnpm", "install", "--offline", "--frozen-lockfile", "--ignore-scripts",
                             "--store-dir", "/root/.local/share/pnpm/store",
                             "--filter", "@chariox/shell...", "--filter", "@chariox/cli..."])
        run("shell-build", ["pnpm", "--filter", "@chariox/shell", "run", "build"])
        run("cli-build", ["pnpm", "--filter", "@chariox/cli", "run", "build"])
        run("rust-build", ["flock", LOCK, "cargo", "+1.88.0", "test", "-p", "chariox-kernel",
                           "--lib", "--locked", "--no-run", "--message-format=json"])
        artifact = None
        for line in (output / "rust-build.log").read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("executable"):
                artifact = Path(item["executable"])
        if not artifact:
            raise RuntimeError("MP-11: no test artifact")
        digest = hashlib.file_digest(artifact.open("rb"), "sha256").hexdigest()
        source_identity((head, tree))
        (output / "artifact.json").write_text(json.dumps({"mp": MP, "head": head, "tree": tree,
            "source_policy": "clean_checkout",
            "base": "74e50b787a5919ee5c3d580c5b088989fd4a1adf", "binary": str(artifact), "sha256": digest}, indent=2))
        environment["CHARIOX_SUDO_SHELL_CLI"] = str(ROOT / "apps/shell/dist/shell.js")
        for name, selector in [
            ("kernel-access", "kernel_access"), ("passkey", "passkey"), ("sudo", "sudo"),
            ("popup-live", "one_passkey_prompt_reaches_every_terminal"),
            ("local-enforcement", "laptop_kernel_websocket_enforces_local_tokens"),
            ("host-enforcement", "kernel_websocket_auth_rejects_missing_or_wrong_tokens"),
            ("provider-launch", "provider_launch"), ("provider-runtime", "provider_runtime::tests::launch"),
            ("managed-isolation", "managed_isolation"), ("pty", "pty::manager::tests"),
            ("protocol", "local::api::tests::protocol_shapes")]:
            run(name, ["flock", LOCK, str(artifact), selector, "--nocapture", "--test-threads=1"])
        node_tests = [
            "packages/kernel-client/dist/ipc-control-response-replay.test.js",
            "packages/kernel-client/dist/ipc-unix-access.test.js",
            "packages/kernel-client/dist/kernel-authorization-request-policy.test.js",
            "packages/kernel-client/dist/websocket-pending-requests.test.js",
            "apps/cli/dist/access-command.test.js", "apps/cli/dist/sudo-command.test.js",
            "apps/cli/dist/passkey-popup-controller.test.js"]
        run("node-focused", ["node", "--test", "--test-concurrency=1", *node_tests])
        run("sandboxed-pty-child", ["flock", LOCK, str(artifact), "sandboxed_pty_survives_launching_thread_exit",
                                    "--nocapture", "--test-threads=1", "--ignored"])
        run("kit-smoke", ["flock", LOCK, "node", "scripts/sudo-kit-smoke.mjs", str(artifact)])
        source_identity((head, tree))
    finally:
        cancellation_deferred = True
        stop_owned(all_owned)
        remaining = [pid for pid, birth in all_owned.items()
                     if (row := identity(pid)) and row[1] == birth and row[2] != "Z"]
        cleanup = {"mp": MP, "state": str(state), "remaining_owned_processes": remaining,
                   "state_removed": False, "results": len(results), "resources": sample(),
                   "cancelled_signal": cancelled_signal}
        if not remaining:
            shutil.rmtree(state)
            cleanup["state_removed"] = True
        (output / "cleanup.json").write_text(json.dumps(cleanup, indent=2))
        if remaining:
            raise RuntimeError("MP-11: owned processes remain; private state retained")
        for sig, handler in previous_handlers.items():
            signal.signal(sig, handler)
        if cancelled_signal:
            raise ValidationCancelled(cancelled_signal)


if __name__ == "__main__":
    try:
        main()
    except ValidationCancelled as error:
        print(str(error), flush=True)
        raise SystemExit(128 + error.signum)

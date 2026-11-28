"""Run hosted Rust tests with a named deadline for serial libtest cases."""

import argparse
import base64
import hashlib
import json
import os
import re
import select
import signal
import subprocess
import sys
import time
from pathlib import Path


TEST_RESULT = re.compile(rb"(?:^|\s)(?:ok|FAILED|ignored(?:,.*)?|bench:.*)\s*$")
TEST_LINE = re.compile(rb"^test (.+?) \.\.\.(.*)$")
LIST_ENTRY = re.compile(r"^(.+): (?:test|benchmark)$")
KERNEL = ["cargo", "test", "--locked", "-p", "chariox-kernel"]


def with_watchdog(command, seconds):
    print("+ " + " ".join(command[:command.index("--") + 1] if "--" in command else command), flush=True)
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True) as process:
        active = None
        partial = b""
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0 and select.select([process.stdout], [], [], remaining)[0]:
            output = os.read(process.stdout.fileno(), 65536)
            if not output:
                try:
                    return process.wait(timeout=max(0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    break
            sys.stdout.buffer.write(output)
            sys.stdout.buffer.flush()
            lines = (partial + output).split(b"\n")
            partial = lines.pop()[:8192]
            previous = active
            for line in lines:
                match = TEST_LINE.match(line)
                if match:
                    active = None if TEST_RESULT.match(match[2]) else match[1].decode(errors="replace")
                elif TEST_RESULT.match(line) or line.startswith(b"test result:"):
                    active = None
            match = TEST_LINE.match(partial)
            if match and not TEST_RESULT.match(match[2]):
                active = match[1].decode(errors="replace")
            if active != previous or active is None:
                deadline = time.monotonic() + seconds
        label = f"Rust test {active}" if active else "Rust test runner before a test name was available"
        print(f"\n::error::{label} timed out after {seconds} seconds", flush=True)
        # Only the new process group belongs to this run, and its leader has not been reaped.
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        except ProcessLookupError:
            pass
        return 124


def kernel_lib_shard(shard, count, seconds, record_inventory=False):
    command = [*KERNEL, "--lib"]
    subprocess.run([*command, "--no-run"], check=True)
    listing = subprocess.check_output([*command, "--", "--list"], text=True)
    tests = sorted(match[1] for line in listing.splitlines() if (match := LIST_ENTRY.fullmatch(line)))
    if len(set(tests)) != len(tests) or len(tests) < count:
        raise ValueError("Kernel library test listing is incomplete or has duplicate names")
    names = tests[shard::count]
    if record_inventory:
        print("KERNEL_LIB_INVENTORY " + json.dumps({
            "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
            "shard": shard, "shards": count, "tests": tests, "selected": names,
        }), flush=True)
    print(f"Kernel library shard {shard + 1}/{count}: {len(names)} of {len(tests)} named tests", flush=True)
    return with_watchdog([*command, "--", "--exact", *names], seconds)


def protocol_snapshots():
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"]):
        raise ValueError("Protocol snapshot generation requires a clean source checkout")
    context = {"uid": os.getuid(), "gid": os.getgid()}
    for key, path in [("confinement", "/proc/self/attr/current"), ("cgroup", "/proc/self/cgroup")]:
        try:
            context[key] = Path(path).read_text().strip()
        except OSError:
            context[key] = None
    context["no_new_privs"] = next((line.split(":", 1)[1].strip()
        for line in Path("/proc/self/status").read_text().splitlines()
        if line.startswith("NoNewPrivs:")), None)
    context["mount_namespace"] = os.readlink("/proc/self/ns/mnt")
    for program in ["node", "pnpm"]:
        context[program] = subprocess.check_output([program, "--version"], text=True).strip()
    print("PROTOCOL_REHASH_CONTEXT " + json.dumps({
        "source": source, **context,
        "initial_pins": json.loads(Path("fixtures/protocol-hash-pins.json").read_text()),
        "rehash_script_sha256": hashlib.sha256(Path("scripts/rehash-protocol-snapshots.mjs").read_bytes()).hexdigest(),
    }), flush=True)
    status = subprocess.call(["pnpm", "protocol:rehash"])
    patch = subprocess.check_output(["git", "diff", "--binary"])
    files = subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines()
    print("PROTOCOL_REHASH_RECEIPT " + json.dumps({
        "source": source, "exit_code": status, "files": files,
        "file_sha256": {file: hashlib.sha256(Path(file).read_bytes()).hexdigest() for file in files},
        "final_pins": json.loads(Path("fixtures/protocol-hash-pins.json").read_text()),
        "patch_sha256": hashlib.sha256(patch).hexdigest(),
        "patch_base64": base64.b64encode(patch).decode(),
    }), flush=True)
    return status


def workspace_targets(workspace_command, seconds):
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))
    kernel = next(package for package in metadata["packages"] if package["name"] == "chariox-kernel")
    selectors = [("--test", "test"), ("--bin", "bin"), ("--example", "example"), ("--bench", "bench")]
    targets = [value for option, kind in selectors for target in kernel["targets"]
               if kind in target["kind"] and target["test"] for value in (option, target["name"])]
    if not targets:
        raise ValueError("No chariox-kernel non-library tests found")
    commands = [
        workspace_command,
        [*KERNEL, "--no-fail-fast", *targets],
        [*KERNEL, "--doc"],
    ]
    status = 0
    for command in commands:
        if "--doc" not in command:
            build = subprocess.run([*command, "--no-run"])
            if build.returncode:
                status = 1
                continue
        if with_watchdog(command, seconds):
            status = 1
    return status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=["kernel", "workspace", "protocol"])
    parser.add_argument("--shard", type=int, default=0)
    parser.add_argument("--shards", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--record-inventory", action="store_true")
    parser.add_argument("--workspace-command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not 0 <= args.shard < args.shards or args.timeout < 1:
        parser.error("require 0 <= shard < shards and a positive timeout")
    os.environ.update(RUST_TEST_THREADS="1", CARGO_TERM_COLOR="never")
    if args.suite == "kernel":
        return kernel_lib_shard(args.shard, args.shards, args.timeout, args.record_inventory)
    if args.suite == "protocol":
        return protocol_snapshots()
    if not args.workspace_command:
        parser.error("workspace suite requires a Cargo workspace test command")
    return workspace_targets(args.workspace_command, args.timeout)


if __name__ == "__main__":
    sys.exit(main())

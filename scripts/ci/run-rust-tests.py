"""Run hosted Rust tests with a named deadline for serial libtest cases."""

import argparse
import json
import os
import re
import select
import signal
import subprocess
import sys
import time


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
            partial = lines.pop()
            for line in lines:
                match = TEST_LINE.match(line)
                if match or TEST_RESULT.match(line) or line.startswith(b"test result:"):
                    active = None
            match = TEST_LINE.match(partial)
            current = match[1].decode(errors="replace") if match and not TEST_RESULT.match(match[2]) else None
            if current != active or active is None:
                deadline = time.monotonic() + seconds
            active = current
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


def kernel_lib_shard(shard, count, seconds):
    command = [*KERNEL, "--lib"]
    subprocess.run([*command, "--no-run"], check=True)
    listing = subprocess.check_output([*command, "--", "--list"], text=True)
    tests = sorted(match[1] for line in listing.splitlines() if (match := LIST_ENTRY.fullmatch(line)))
    if len(set(tests)) != len(tests) or len(tests) < count:
        raise ValueError("Kernel library test listing is incomplete or has duplicate names")
    names = tests[shard::count]
    print(f"Kernel library shard {shard + 1}/{count}: {len(names)} of {len(tests)} named tests", flush=True)
    return with_watchdog([*command, "--", "--exact", *names], seconds)


def workspace_targets(seconds):
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]))
    kernel = next(package for package in metadata["packages"] if package["name"] == "chariox-kernel")
    selectors = [("--test", "test"), ("--bin", "bin"), ("--example", "example"), ("--bench", "bench")]
    targets = [value for option, kind in selectors for target in kernel["targets"]
               if kind in target["kind"] and target["test"] for value in (option, target["name"])]
    if not targets:
        raise ValueError("No chariox-kernel non-library tests found")
    commands = [
        ["cargo", "test", "--locked", "--workspace", "--exclude", "chariox-kernel", "--no-fail-fast"],
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
    parser.add_argument("suite", choices=["kernel", "workspace"])
    parser.add_argument("--shard", type=int, default=0)
    parser.add_argument("--shards", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if not 0 <= args.shard < args.shards or args.timeout < 1:
        parser.error("require 0 <= shard < shards and a positive timeout")
    os.environ.update(RUST_TEST_THREADS="1", CARGO_TERM_COLOR="never")
    if args.suite == "kernel":
        return kernel_lib_shard(args.shard, args.shards, args.timeout)
    return workspace_targets(args.timeout)


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Run a command on an owned QEMU guest-agent socket; never uses SSH/auth files."""
import argparse
import base64
import json
import socket
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command or args.timeout < 1:
        parser.error("supply a command and a positive timeout")
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(15)
        connection.connect(args.socket)
        reader = connection.makefile("rb")

        def request(name, arguments=None):
            message = {"execute": name}
            if arguments is not None:
                message["arguments"] = arguments
            connection.sendall(json.dumps(message).encode() + b"\n")
            response = json.loads(reader.readline())
            if "error" in response:
                raise RuntimeError(response["error"])
            return response["return"]

        request("guest-ping")
        result = request("guest-exec", {
            "path": command[0], "arg": command[1:], "capture-output": True,
            "input-data": base64.b64encode(sys.stdin.buffer.read()).decode(),
        })
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            status = request("guest-exec-status", {"pid": result["pid"]})
            if status["exited"]:
                for name, stream in [("out", sys.stdout.buffer), ("err", sys.stderr.buffer)]:
                    stream.write(base64.b64decode(status.get(name + "-data", "")))
                    stream.flush()
                if status.get("out-truncated") or status.get("err-truncated"):
                    raise RuntimeError("guest output truncated; collect a smaller receipt")
                return status.get("exitcode", 128 + status.get("signal", 0))
            time.sleep(1)
        # A timeout is not cancellation: leave the PID visible for owned cleanup.
        raise TimeoutError(f"guest command pid {result['pid']} exceeded {args.timeout}s")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        print(f"guest_command_failed: {error}", file=sys.stderr)
        sys.exit(1)

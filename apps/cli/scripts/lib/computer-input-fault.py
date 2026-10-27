#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: fatal child fault through production Bash/native input."""
import importlib.util
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import time

spec = importlib.util.spec_from_file_location("owned", Path(__file__).with_name("computer-input-process.py"))
owned = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owned)
from selkies.Xlib import display


def held():
    connection = display.Display()
    try:
        return {"keys": sum(n.bit_count() for n in connection.query_keymap()),
                "buttons": connection.screen().root.query_pointer().mask & 7936}
    finally:
        connection.close()


def main():
    witness = Path(os.environ["CHARIOX_SLICE_PRIVATE_ROOT"]) / "fatal-child.json"
    if len(sys.argv) == 3 and sys.argv[1] == "assert-released":
        proof = json.loads(witness.read_text())
        if proof["action"] != sys.argv[2] or proof["status"] != 131 or not proof["resetDone"]:
            raise ValueError("fatal child/reset witness missing")
        if held() != {"keys": 0, "buttons": 0}:
            raise ValueError("native input remains held at failed acknowledgement")
        return 0
    args = sys.argv[1:]
    if args[0] == "computer-input-reset":
        status = subprocess.call(["bash", "/opt/computer-source/slice-screen.sh", *args])
        if status == 0 and witness.exists():
            proof = json.loads(witness.read_text())
            proof["resetDone"] = True
            witness.write_text(json.dumps(proof))
        return status
    if args[0] not in ("computer-key-hold-stdin", "pointer-hold"):
        raise ValueError("unsupported fault action")
    witness.unlink(missing_ok=True)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    with subprocess.Popen(["bash", "/opt/computer-source/slice-screen.sh", *args], stdin=subprocess.PIPE) as parent:
        identity = {"pid": parent.pid, "start": owned.process_stat(parent.pid)["start"]}
        parent.stdin.write(sys.stdin.buffer.read() if args[0] == "computer-key-hold-stdin" else b"")
        parent.stdin.close()
        deadline = time.monotonic() + 5
        try:
            while time.monotonic() < deadline:
                native = held()
                if (native["keys"] if args[0] == "computer-key-hold-stdin" else native["buttons"]):
                    break
                if parent.poll() is not None:
                    raise ValueError("helper exited before physical press")
                time.sleep(.02)
            else:
                raise ValueError("physical press timeout")
            children = []
            for path in Path("/proc").iterdir():
                if path.name.isdigit():
                    try:
                        info = owned.process_stat(int(path.name))
                        if info["parent"] == parent.pid:
                            children.append({"pid": int(path.name), "start": info["start"]})
                    except OSError:
                        pass
            if len(children) != 1:
                raise ValueError("ambiguous physical helper child")
            owned.signal_owned_child(children[0], identity, signal.SIGQUIT)
            status = parent.wait(timeout=5)
            if status != 131:
                raise ValueError("Bash did not preserve SIGQUIT numeric exit")
            remaining = held()
            if remaining == {"keys": 0, "buttons": 0}:
                raise ValueError("fault did not leave input for kernel reset")
            witness.write_text(json.dumps({"action": args[0], "status": status,
                                           "heldAtDeath": remaining, "resetDone": False}))
            return status
        finally:
            if parent.poll() is None:
                owned.signal_owned_child(identity, {"pid": os.getpid(), "start": owned.process_stat(os.getpid())["start"]}, signal.SIGTERM)
                parent.wait(timeout=12)


if __name__ == "__main__":
    raise SystemExit(main())

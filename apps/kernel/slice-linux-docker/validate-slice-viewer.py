#!/usr/bin/env python3
"""Run in slice-runtime-deps to test the real desktop launcher, without providers."""

import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


def main():
    source = Path(__file__).parent / "docker"
    with tempfile.TemporaryDirectory(prefix="chariox-viewer-drill-") as scratch:
        root = Path(scratch)
        for name in ("slice-screen.sh", "slice-selkies.py", "browser-cdp.mjs"):
            shutil.copy2(source / name, root / name)
        environment = {
            **os.environ, "HOME": scratch, "XDG_RUNTIME_DIR": scratch,
            "CHARIOX_SLICE_ROOT": scratch, "CHARIOX_SLICE_DISPLAY": ":92",
            "CHARIOX_SLICE_SCREEN_GEOMETRY": "640x480x24",
            "CHARIOX_SLICE_VIEWER_BACKEND": "selkies", "OMP_NUM_THREADS": "1",
            "CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS": "",
        }

        def screen(action, *args, expected=0):
            result = subprocess.run(["bash", str(root / "slice-screen.sh"), action, *args],
                                    env=environment, capture_output=True, text=True, timeout=50)
            assert result.returncode == expected, (action, result.stdout, result.stderr)
            return result.stdout

        try:
            assert "available=true" in screen("start")
            assert "viewer=http://127.0.0.1:6080/\n" in screen("status")
            current = subprocess.run([sys.executable, str(root / "slice-selkies.py"), "status"],
                                     env=environment, capture_output=True, text=True, check=True)
            pid = json.loads(current.stdout)["pid"]
            os.kill(pid, signal.SIGKILL)
            time.sleep(0.2)
            assert "missing=selkies" in screen("status", expected=1)
            # Display streaming is not an admission prerequisite for Browser tools.
            result = subprocess.run(["node", str(root / "browser-cdp.mjs"), "status"],
                                    env=environment, capture_output=True, text=True, timeout=10)
            assert result.returncode == 0, result.stderr
            shot = root / "display.png"
            screen("screenshot", str(shot))
            assert shot.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
            screen("stop")
            assert not Path("/tmp/.X11-unix/X92").exists()

            # A broken Selkies launch must fail, never silently launch noVNC.
            environment["CHARIOX_SLICE_SELKIES_BIN"] = "/bin/false"
            screen("start", expected=1)
            with socket.socket() as probe:
                assert probe.connect_ex(("127.0.0.1", 6080)) != 0
            assert not Path("/tmp/.X11-unix/X92").exists()
            environment.pop("CHARIOX_SLICE_SELKIES_BIN")

            # Explicit rollback uses the existing noVNC launcher and cleanup.
            environment["CHARIOX_SLICE_VIEWER_BACKEND"] = "novnc"
            assert "available=true" in screen("start")
            assert "/vnc.html?" in screen("status")
            screen("stop")
            assert not Path("/tmp/.X11-unix/X92").exists()
            for port in (5900, 6080, 9222):
                with socket.socket() as probe:
                    assert probe.connect_ex(("127.0.0.1", port)) != 0, port
            print(json.dumps({"desktop_lifecycle": "pass", "browser_survives_streamer_crash": "pass",
                              "silent_fallback": "absent", "explicit_novnc_rollback": "pass",
                              "cleanup": "pass"}))
        except BaseException:
            for log in (root / "logs").glob("*.log"):
                print(f"{log.name}:\n{log.read_text(errors='replace')[-1500:]}", flush=True)
            raise
        finally:
            subprocess.run(["bash", str(root / "slice-screen.sh"), "stop"], env=environment,
                           capture_output=True, timeout=50)


if __name__ == "__main__":
    main()

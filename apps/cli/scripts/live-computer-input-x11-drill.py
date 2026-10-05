#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: viewer-independent physical helper matrix, not MP closure.

Requires a disposable headed fixture image with Xorg dummy/Xvfb and Mousepad.
No providers, owner credentials, published ports, viewer or Room authority.
"""
import argparse
import hashlib
import json
import os
import signal
from pathlib import Path
import subprocess
import threading
import time
import uuid

REPO = Path(__file__).resolve().parents[3]
SOURCE = REPO / "apps/kernel/slice-linux-docker/docker"
PYTHON = "/opt/chariox-selkies/bin/python"
ROOT = "/tmp/chariox-computer-fixture"
CDP = """
const chunks=[];for await(const c of process.stdin)chunks.push(c);
const pages=await(await fetch('http://127.0.0.1:9222/json/list')).json();
const page=pages.find(p=>p.type==='page' && p.url.includes('computer-input-fixture.html'));
if(!page)throw new Error('fixture tab missing');
const ws=new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j});
ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:Buffer.concat(chunks).toString(),returnByValue:true}}));
const reply=await new Promise((r,j)=>{ws.onmessage=e=>{const d=JSON.parse(e.data);if(d.id===1)r(d)};ws.onerror=j});
ws.close();if(reply.error||reply.result?.exceptionDetails)throw new Error('fixture observation failed');
process.stdout.write(JSON.stringify({tabId:page.id,url:page.url,value:reply.result.result.value}));
"""


class DrillInterrupted(Exception):
    # InterruptedError is an OSError that subprocess/IO can retry internally.
    # A dedicated exception must escape those retry loops to reach cleanup.
    pass


def resources():
    memory = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    disk = os.statvfs("/")
    return {"at": time.time(), "memAvailableBytes": int(memory["MemAvailable"].split()[0]) * 1024,
            "diskFreeBytes": disk.f_bavail * disk.f_frsize, "load": list(os.getloadavg())}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--server", choices=["Xorg", "Xvfb"], default="Xorg")
    args = parser.parse_args()
    evidence = args.output.resolve()
    if not args.output.is_absolute() or evidence.is_relative_to(REPO):
        parser.error("MP-08/MP-10/MP-11 evidence must be absolute and outside the repository")
    evidence.mkdir(parents=True, exist_ok=False)
    name = "chariox-computer-fixture-" + uuid.uuid4().hex[:12]
    report = {"items": ["MP-08", "MP-10", "MP-11"], "startedAt": time.time(), "cases": [],
              "commands": [], "resources": [], "server": args.server, "container": name,
              "scope": "physical shared helpers only; no official-provider/Web/TUI/Room/managed acceptance"}
    stop = threading.Event()
    watcher = None

    def interrupted(signum, _frame):
        raise DrillInterrupted(f"MP-08/MP-10/MP-11 drill interrupted by signal {signum}")

    for signum in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signum, interrupted)

    def run(command, stdin=None, accepted=(0,), timeout=40):
        start = time.monotonic()
        result = subprocess.run(command, input=stdin, capture_output=True, timeout=timeout)
        report["commands"].append({"argv": command, "exitCode": result.returncode,
                                   "elapsedSeconds": time.monotonic() - start})
        if result.returncode not in accepted:
            # Command stdout/stderr may contain clipboard or text. Never retain it.
            raise RuntimeError(f"{command[0]} operation exited {result.returncode}")
        return result.stdout

    def docker(*command, **kwargs):
        return run(["docker", *command], **kwargs)

    def execute(*command, **kwargs):
        return docker("exec", "-i", "-u", "slice", "-e", "DISPLAY=:99", "-e",
                      f"CHARIOX_SLICE_DISPLAY_SERVER={args.server}", "-e", "CHARIOX_SLICE_ROOT=/opt/computer-source",
                      "-e", f"CHARIOX_SLICE_PRIVATE_ROOT={ROOT}", "-e", f"CHARIOX_SLICE_CHROME_PROFILE={ROOT}/profile",
                      name, *command, **kwargs)

    def screen(*command, **kwargs):
        return execute("bash", "/opt/computer-source/slice-screen.sh", *map(str, command), **kwargs)

    def receipt():
        return json.loads(execute("node", "--input-type=module", "-e", CDP,
                                 stdin=b"window.computerReceipt()"))

    def wait(check, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                value = check()
                if value:
                    return value
            except (RuntimeError, ValueError):
                pass
            time.sleep(.1)
        raise AssertionError("physical acknowledgement did not arrive within bound")

    def case(label, action):
        start = time.monotonic()
        try:
            detail = action() or {}
            report["cases"].append({"id": label, "status": "PASS", "detail": detail})
        except DrillInterrupted:
            raise
        except Exception as error:
            report["cases"].append({"id": label, "status": "RED", "failure": str(error)})
        report["cases"][-1]["elapsedSeconds"] = time.monotonic() - start

    def guard():
        while not stop.wait(5):
            sample = resources()
            report["resources"].append(sample)
            if sample["memAvailableBytes"] < 16 * 1024**3 or sample["diskFreeBytes"] < 10 * 1024**3:
                report["resourceStop"] = sample
                subprocess.run(["docker", "stop", "-t", "2", name], capture_output=True, timeout=10)
                return

    try:
        sample = resources()
        report["resources"].append(sample)
        assert sample["memAvailableBytes"] > 18 * 1024**3, "16 GiB memory floor plus 2 GiB fixture forecast"
        assert sample["diskFreeBytes"] > 11 * 1024**3, "10 GiB disk floor plus fixture forecast"
        report["sourceCommit"] = run(["git", "-C", str(REPO), "rev-parse", "HEAD"]).decode().strip()
        report["sourceDirty"] = bool(run(["git", "-C", str(REPO), "status", "--porcelain"]).strip())
        report["sourceDiffSha256"] = hashlib.sha256(run(["git", "-C", str(REPO), "diff", "HEAD"])).hexdigest()
        report["helperSha256"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in
                                  [SOURCE / "slice-screen.sh", SOURCE / "slice-keyboard.py", SOURCE / "slice-text-finder.py",
                                   SOURCE / "slice-observation-mask.py", Path(__file__)]}
        fixture_html = Path(__file__).with_name("lib") / "computer-input-fixture.html"
        report["fixtureSha256"] = hashlib.sha256(fixture_html.read_bytes()).hexdigest()
        report["imageId"] = docker("image", "inspect", "--format", "{{.Id}}", args.image).decode().strip()
        docker("run", "-d", "--name", name, "--label", "chariox.drill=computer-input", "--network", "none",
               "--memory", "2g", "--memory-swap", "2g", "--cpus", "1", "--pids-limit", "512",
               "--security-opt", f"seccomp={SOURCE.parent}/chromium-seccomp.json",
               "--mount", f"type=bind,source={SOURCE},target=/opt/computer-source,readonly",
               "--entrypoint", "sleep", args.image, "infinity")
        watcher = threading.Thread(target=guard, daemon=True)
        watcher.start()
        execute("mkdir", "-p", ROOT)
        docker("cp", str(fixture_html), f"{name}:{ROOT}/computer-input-fixture.html")
        docker("exec", "-u", "root", name, "chown", "slice:slice", f"{ROOT}/computer-input-fixture.html")
        if args.server == "Xorg":
            docker("exec", "-d", "-u", "root", name, "Xorg", ":99", "-config", "/opt/computer-source/xorg-dummy.conf",
                   "-logfile", f"{ROOT}/Xorg.log", "-nolisten", "tcp", "-noreset", "-ac")
        else:
            docker("exec", "-d", "-u", "slice", name, "Xvfb", ":99", "-screen", "0", "1280x800x24", "-ac", "+extension", "XTEST")
        wait(lambda: execute("xdpyinfo", accepted=(0, 1)))
        execute("xrandr", "--output", "DUMMY0", "--mode", "1280x800", accepted=(0, 1)) if args.server == "Xorg" else None
        docker("exec", "-d", "-u", "slice", "-e", "DISPLAY=:99", name, "openbox")
        execute("mkdir", "-p", f"{ROOT}/profile", f"{ROOT}/runtime")
        execute("chmod", "0700", f"{ROOT}/runtime")
        docker("exec", "-d", "-u", "slice", "-e", "DISPLAY=:99", "-e", f"XDG_RUNTIME_DIR={ROOT}/runtime", name,
               "chromium", "--disable-gpu", "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check",
               f"--user-data-dir={ROOT}/profile", "--password-store=basic", "--remote-debugging-port=9222",
               "--start-maximized", f"file://{ROOT}/computer-input-fixture.html")
        initial = wait(receipt)
        report["versions"] = {
            "chromium": execute("chromium", "--version").decode().strip(),
            "xdotool": execute("xdotool", "version").decode().strip(),
            "tesseract": execute("tesseract", "--version").decode().splitlines()[0],
            "packages": execute("dpkg-query", "-W", "-f=${Package} ${Version}\n", "mousepad",
                                "xvfb", "xserver-xorg-core", "xserver-xorg-video-dummy", "tesseract-ocr-eng",
                                "tesseract-ocr-deu").decode().splitlines(),
            "hostKernel": os.uname().release,
        }
        report["tabId"] = initial["tabId"]
        report["documentId"] = initial["value"]["documentId"]
        # Window origin comes from the rendered screen, not assumed browser chrome height.
        origin = json.loads(execute("node", "--input-type=module", "-e", CDP,
                                   stdin=b"[screenX,screenY+outerHeight-innerHeight]"))["value"]

        def point(x, y):
            return round(x + origin[0]), round(y + origin[1])

        def prepare():
            screen("pointer-click", *point(100, 120), "left", 1)
            screen("computer-key-stdin", 1, stdin=b"ctrl+a")
            screen("computer-key-stdin", 1, stdin=b"BackSpace")

        def text_check(layout, text):
            execute("setxkbmap", layout)
            prepare()
            before = receipt()
            screen("computer-type-stdin", stdin=text.encode(), timeout=120)
            after = receipt()
            assert before["value"]["documentId"] == after["value"]["documentId"], "physical Unicode reloaded the browser document"
            assert after["value"]["value"] == text.replace("\r", "\n"), "physical text differs from requested Unicode"
            assert after["value"]["focused"] == "TEXTAREA", "text helper lost native editor focus"
            assert any(e["kind"] == "input" and e["trusted"] for e in after["value"]["events"]), "no trusted physical input acknowledgement"
            return {"layout": layout, "characters": len(text), "sameTab": after["tabId"] == initial["tabId"], "sameDocument": True}

        for layout, text in [("us", "Grüße 世界 áéíóú Ж\nsecond line\n"), ("de", "QWERTZ @ € Grüße\n"),
                             ("us", "".join(chr(0x4e00 + i) for i in range(96))), ("us", "Cafe\u0301\n")]:
            case("keyboard." + layout + "." + str(len(text)), lambda layout=layout, text=text: text_check(layout, text))
        execute("setxkbmap", "us")

        def inherited_overlay_check():
            prepare()
            before = receipt()
            # Another keyboard handler may leave an overlay on an otherwise
            # spare hardware code. Seed its exact shape on BrowserRefresh.
            execute(PYTHON, "-c", "from selkies.Xlib import display; d=display.Display(); "
                    "assert 181 not in {kc for row in d.get_modifier_mapping() for kc in row}; "
                    "d.change_keyboard_mapping(181, [[0x01004e16, 0x01004e16]]); d.sync(); d.close()")
            try:
                screen("computer-type-stdin", stdin="世".encode())
                def acknowledged():
                    current = receipt()
                    return current if current["value"]["documentId"] != before["value"]["documentId"] or current["value"]["value"] == "世" else None
                after = wait(acknowledged)
                assert after["value"]["documentId"] == before["value"]["documentId"], "inherited unsafe overlay reloaded the browser document"
                assert after["value"]["value"] == "世", "inherited unsafe overlay bypassed safe text allocation"
                events = after["value"]["events"][len(before["value"]["events"]):]
                assert any(e["kind"] == "input" and e["trusted"] for e in events), "inherited overlay lacks trusted input acknowledgement"
                assert not any(e.get("key") == "BrowserRefresh" or e.get("code") == "BrowserRefresh" for e in events), "unsafe hardware code reached Chromium"
                assert after["value"]["focused"] == "TEXTAREA", "inherited overlay changed editor focus"
                return {"inheritedKeycode": 181, "sameDocument": True, "trustedText": True, "focusRetained": True}
            finally:
                execute("setxkbmap", "us")
        case("keyboard.inherited-unsafe-overlay", inherited_overlay_check)

        def pointer_check():
            before = len(receipt()["value"]["events"])
            screen("pointer-click", *point(100, 280), "left", 1)
            screen("pointer-click", *point(100, 280), "left", 2)
            screen("pointer-click", *point(100, 280), "right", 1)
            def clicked():
                current = receipt()["value"]
                events = current["events"][before:]
                return current if any(e["kind"] == "contextmenu" for e in events) else None
            after_clicks = wait(clicked)
            events = after_clicks["events"][before:]
            clicks = sum(e["kind"] == "click" and e["button"] == 0 for e in events)
            assert clicks == 3, f"single/double-click effect counts differ: {clicks}"
            assert sum(e["kind"] == "dblclick" for e in events) == 1, "double-click not acknowledged exactly once"
            assert sum(e["kind"] == "contextmenu" for e in events) == 1, "right-click not acknowledged exactly once"
            # A drag release within this target can also dispatch a click.
            # Count the single/double clicks before delivering the drag.
            drag_start = len(after_clicks["events"])
            screen("pointer-drag", *point(100, 280), *point(240, 300), "left")
            screen("pointer-scroll", *point(500, 280), 3, 3)
            def scrolled():
                current = receipt()["value"]
                return current if current["scrollX"] > 0 and current["scrollY"] > 0 else None
            after = wait(scrolled)
            drag_events = after["events"][drag_start:]
            assert sum(e["kind"] == "mousedown" and e["button"] == 0 for e in drag_events) == 1, "drag initial press differs"
            releases = sum(e["kind"] == "mouseup" and e["button"] == 0 for e in drag_events)
            assert releases == 1, f"drag release differs: {releases}"
            drag_clicks = sum(e["kind"] == "click" and e["button"] == 0 for e in drag_events)
            assert drag_clicks <= 1, "drag release produced duplicate clicks"
            assert any(e["kind"] == "mousemove" and e["buttons"] == 1 for e in drag_events), "drag did not retain held button"
            assert after["scrollX"] > 0 and after["scrollY"] > 0, "both scroll axes must move"
            assert all(e["trusted"] for e in after["events"][before:]), "untrusted pointer event"
            return {"clicks": 3, "doubleClicks": 1, "rightClicks": 1, "dragPresses": 1,
                    "dragReleases": 1, "dragReleaseClicks": drag_clicks, "scrollAxes": 2}
        case("pointer.click-double-right-drag-scroll", pointer_check)

        def clipboard_check():
            prepare()
            text = "Clipboard Grüße 世界\n"
            screen("computer-clipboard-write-stdin", stdin=text.encode())
            screen("computer-key-stdin", 1, stdin=b"ctrl+v")
            assert receipt()["value"]["value"] == text, "agent-to-application paste differs"
            screen("computer-key-stdin", 1, stdin=b"ctrl+a")
            screen("computer-key-stdin", 1, stdin=b"ctrl+c")
            assert screen("computer-clipboard-read").decode() == text, "application-to-human clipboard differs"
            return {"agentToApplication": True, "applicationToHuman": True,
                    "agentReadTool": "not exercised; helper read is a human/evaluator path"}
        case("clipboard.directions", clipboard_check)

        def selection_check():
            prepare()
            screen("computer-type-stdin", stdin=b"Select this physical text")
            before = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            screen("pointer-drag", *point(47, 100), *point(270, 100), "left")
            selected = json.loads(execute("node", "--input-type=module", "-e", CDP,
                stdin=b"(()=>{const e=document.querySelector('textarea');return e.value.slice(e.selectionStart,e.selectionEnd)})()"))["value"]
            assert selected, "physical drag did not select text"
            screen("computer-key-stdin", 1, stdin=b"ctrl+c")
            assert screen("computer-clipboard-read").decode() == selected, "copied text differs from physical selection"
            assert json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target")) == before, "selection moved native window/pane"
            screen("computer-key-stdin", 3, stdin=b"Right")
            after = receipt()["value"]["events"]
            assert sum(e["kind"] == "keydown" and e["key"] == "ArrowRight" for e in after) == 3, "repeated arrow chord count differs"
            return {"selectedCharacters": len(selected), "copiedSelection": True, "nativeGeometryRetained": True, "arrowRepeats": 3}
        case("pointer.text-selection-key-repeat", selection_check)

        def native_held():
            return json.loads(execute(PYTHON, "-c", "from selkies.Xlib import display; import json; d=display.Display(); print(json.dumps({'keys':sum(n.bit_count() for n in d.query_keymap()),'buttons':d.screen().root.query_pointer().mask & 7936})); d.close()"))

        def hold_check(kind):
            prepare()
            focus = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            before = len(receipt()["value"]["events"])
            if kind == "key":
                screen("computer-key-hold-stdin", 750, stdin=b"shift+F8")
                down, up, field, value = "keydown", "keyup", "key", "F8"
            else:
                screen("pointer-hold", *point(100, 280), "left", 750)
                down, up, field, value = "mousedown", "mouseup", "button", 0
            events = receipt()["value"]["events"][before:]
            presses = [e for e in events if e["kind"] == down and e.get(field) == value and not e.get("repeat")]
            releases = [e for e in events if e["kind"] == up and e.get(field) == value]
            assert len(presses) == len(releases) == 1, "hold must acknowledge one initial press and release"
            elapsed = releases[0]["at"] - presses[0]["at"]
            assert elapsed >= 650, "hold released before requested duration"
            assert all(e["trusted"] for e in presses + releases), "hold input was not physical"
            assert native_held() == {"keys": 0, "buttons": 0}, "hold left native input pressed"
            if kind == "key":
                assert json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target")) == focus, "keyboard hold changed native focus"
            return {"initialPresses": 1, "releases": 1, "durationMs": elapsed, "heldAfter": native_held(), "trusted": True}
        case("keyboard.hold-release", lambda: hold_check("key"))
        case("pointer.hold-release", lambda: hold_check("button"))

        def hold_cancel_check(signum):
            prepare()
            code = "import os; os.getpgrp()==os.getpid() or os.setsid(); open('" + ROOT + "/hold-pgid','w').write(str(os.getpid())); os.execv('/bin/bash',['bash','/opt/computer-source/slice-screen.sh','computer-key-hold-stdin','10000'])"
            result = []
            def held():
                try:
                    execute(PYTHON, "-c", code, stdin=b"shift+F8", accepted=(0, 137, 143), timeout=20)
                    result.append(True)
                except Exception as error:
                    result.append(error)
            worker = threading.Thread(target=held)
            before = len(receipt()["value"]["events"])
            worker.start()
            wait(lambda: native_held()["keys"] == 2)
            execute(PYTHON, "-c", "import os,signal; pid=int(open('" + ROOT + "/hold-pgid').read()); assert os.getpgid(pid)==pid; os.killpg(pid," + str(signum) + ")")
            worker.join(10)
            assert not worker.is_alive() and result == [True], "interrupted hold did not settle"
            if signum == signal.SIGKILL:
                # Existing kernel Action cancellation seam resets after process-group settlement.
                screen("computer-input-reset")
            wait(lambda: native_held() == {"keys": 0, "buttons": 0})
            events = receipt()["value"]["events"][before:]
            assert any(e["kind"] == "keyup" and e["key"] == "F8" and e["trusted"] for e in events), "cancel release acknowledgement missing"
            time.sleep(.2)
            assert native_held() == {"keys": 0, "buttons": 0}, "cancelled hold pressed input again"
            return {"signal": int(signum), "released": True, "resetAfterKill": signum == signal.SIGKILL}
        case("keyboard.hold-interrupt-release", lambda: hold_cancel_check(signal.SIGTERM))
        case("keyboard.hold-cancel-reset", lambda: hold_cancel_check(signal.SIGKILL))

        def rejected_hold_check():
            for duration in [0, 10001]:
                screen("computer-key-hold-stdin", duration, stdin=b"shift+F8", accepted=(1,))
            screen("computer-key-hold-stdin", 100, stdin=b"NoSuchFixtureKey", accepted=(1,))
            execute("xdotool", "keydown", "Shift_L")
            before = native_held()
            try:
                screen("computer-key-hold-stdin", 100, stdin=b"F8", accepted=(1,))
                assert native_held() == before, "failed hold released pre-existing foreign input"
            finally:
                screen("computer-input-reset")
            return {"durationBounds": True, "unknownKeyRejected": True, "foreignHoldPreserved": True}
        case("keyboard.hold-rejections", rejected_hold_check)

        def cancellation_check():
            prepare()
            # Kernel cancellation kills the owned process group and resets input.
            code = "import os; os.getpgrp()==os.getpid() or os.setsid(); open('" + ROOT + "/typing-pgid','w').write(str(os.getpid())); os.execv('/bin/bash',['bash','/opt/computer-source/slice-screen.sh','computer-type-stdin'])"
            future = []
            def type_held():
                try:
                    future.append(execute(PYTHON, "-c", code, stdin=b"x" * 400, accepted=(0, 137), timeout=40))
                except Exception as error:
                    future.append(error)
            worker = threading.Thread(target=type_held)
            worker.start()
            wait(lambda: 0 < len(receipt()["value"]["value"]) < 400)
            execute(PYTHON, "-c", "import os,signal; pid=int(open('" + ROOT + "/typing-pgid').read()); assert os.getpgid(pid)==pid; os.killpg(pid,signal.SIGKILL)")
            worker.join(10)
            assert not worker.is_alive(), "cancelled helper did not settle"
            screen("computer-input-reset")
            size = len(receipt()["value"]["value"])
            time.sleep(.5)
            assert len(receipt()["value"]["value"]) == size, "physical typing continued after cancel"
            held = int(execute(PYTHON, "-c", "from selkies.Xlib import display; d=display.Display(); print(sum(n.bit_count() for n in d.query_keymap())); d.close()"))
            assert held == 0, "keys remained down after reset"
            # Ctrl+Shift+A opens Chromium's tab-search UI. Seed a held key
            # without invoking a browser command in this reset-only fixture.
            execute("xdotool", "keydown", "Shift_L", "Control_L", "F8", "mousedown", "1")
            screen("computer-input-reset")
            reset = json.loads(execute(PYTHON, "-c", "from selkies.Xlib import display; import json; d=display.Display(); print(json.dumps({'keys':sum(n.bit_count() for n in d.query_keymap()),'buttons':d.screen().root.query_pointer().mask & 7936})); d.close()"))
            assert reset == {"keys": 0, "buttons": 0}, "held native keys/buttons remained after reset"
            return {"settledCharacters": size, "heldKeys": held, "nativeHeldReset": reset}
        case("keyboard.cancel-reset", cancellation_check)

        def observation_check():
            target = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            screen("protected-screenshot", f"{ROOT}/screen.png", stdin=b'{"unknown":false,"targets":[],"values":[]}')
            assert json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target")) == target, "screenshot changed native focus"
            matches = [json.loads(line) for line in screen("find-text", "Open Room", f"{ROOT}/screen.png").splitlines()]
            assert len(matches) == 2, "real OCR must find two distinct visible labels"
            expected = receipt()["value"]["geometry"]["ocr"]
            assert all(point(expected[0], expected[1])[0] <= m["center_x"] <= point(expected[0] + expected[2], 0)[0] for m in matches), "OCR coordinates escaped rendered text"
            missing = screen("find-text", "NonexistentFixtureLabel", f"{ROOT}/screen.png", accepted=(1,))
            assert missing.strip() == b"null", "no-match did not return null"
            assert matches[1]["height"] > matches[0]["height"], "scaled text must retain its larger original-pixel geometry"
            non_english = screen("find-text", "Straße", f"{ROOT}/screen.png")
            assert len(non_english.splitlines()) == 1, "real non-English OCR sample missing"
            inspect = json.loads(execute(PYTHON, "-c", "from PIL import Image; import json; i=Image.open('" + ROOT + "/screen.png'); print(json.dumps({'size':i.size,'marker':i.getpixel((" + str(point(80, 300)[0]) + "," + str(point(80, 300)[1]) + "))})); i.close()"))
            assert inspect["size"] == [1280, 800], "screenshot geometry differs from active display"
            assert inspect["marker"][:3] == [34, 170, 68], "screenshot misses active physical surface marker"
            docker("cp", f"{name}:{ROOT}/screen.png", str(evidence / "screen.png"))
            data = (evidence / "screen.png").read_bytes()
            assert len(data) <= 16 * 1024**2, "screenshot exceeds inline model byte limit"
            return {"geometry": inspect["size"], "sha256": hashlib.sha256(data).hexdigest(), "sizeBytes": len(data),
                    "ocrMatches": len(matches), "nativeFocusRetained": True, "modelDelivery": "NOT_RUN"}
        case("observe.screenshot-ocr", observation_check)

        def protected_observation_check():
            prepare()
            # Public static input/media canary only. Vault insertion and
            # authorization are not established by this supplied policy.
            text = "Private Marker"
            focus = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            # Locate the actual public canary pixels using real OCR. A value
            # policy masks echoed text bounds, not every blank input pixel.
            screen("protected-screenshot", f"{ROOT}/canary-baseline.png",
                   stdin=b'{"unknown":false,"targets":[],"values":[]}')
            raw_matches = [json.loads(line) for line in screen("find-text", text, f"{ROOT}/canary-baseline.png").splitlines()]
            assert len(raw_matches) == 2, "public canary must render in both the field and opaque media"
            boxes = [(m["left"], m["top"], m["left"] + m["width"], m["top"] + m["height"]) for m in raw_matches]
            policy = json.dumps({"unknown": False, "targets": [], "values": [text]}).encode()
            screen("protected-screenshot", f"{ROOT}/protected.png", stdin=policy)
            secret_region = receipt()["value"]["geometry"]["secret"]
            sx, sy = point(secret_region[0], secret_region[1])
            # Decode the PNG, check every opaque-media pixel and both rendered
            # text boxes, then check a benign marker outside protected regions.
            inspect = json.loads(execute(PYTHON, "-c", "from PIL import Image; import json; "
                "i=Image.open('" + ROOT + "/protected.png').convert('RGB'); "
                "box=" + repr((sx, sy, sx + round(secret_region[2]), sy + round(secret_region[3]))) + "; "
                "print(json.dumps({'size':i.size,'opaqueBlack':i.crop(box).getextrema()==((0,0),(0,0),(0,0)),"
                "'textBoxesBlack':all(i.crop(b).getextrema()==((0,0),(0,0),(0,0)) for b in " + repr(boxes) + "),"
                "'benign':i.getpixel(" + repr(point(80, 300)) + ")})); i.close()"))
            assert inspect["size"] == [1280, 800], "protected screenshot changed canonical geometry"
            assert inspect["opaqueBlack"] and inspect["textBoxesBlack"], "protected text or opaque-media pixels escaped masking"
            assert inspect["benign"] == [34, 170, 68], "redaction obscured unrelated marker pixels"
            missing = screen("protected-find-text", text, stdin=policy, accepted=(1,))
            assert missing.strip() == b"null", "protected text escaped real OCR masking"
            benign = screen("protected-find-text", "Open Room", stdin=policy)
            assert len(benign.splitlines()) == 2, "redacted OCR lost distinct benign matches"
            assert json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target")) == focus, "protected observation changed native focus"
            docker("cp", f"{name}:{ROOT}/protected.png", str(evidence / "protected.png"))
            data = (evidence / "protected.png").read_bytes()
            assert len(data) <= 16 * 1024**2, "protected screenshot exceeds image byte limit"
            return {"geometry": inspect["size"], "opaqueMediaMasked": True, "inputMasked": True,
                    "benignPixelsRetained": True, "protectedOcrMatches": 0, "benignOcrMatches": 2,
                    "nativeFocusRetained": True, "sha256": hashlib.sha256(data).hexdigest(),
                    "sizeBytes": len(data), "modelDelivery": "NOT_RUN", "vaultAuthorization": "NOT_RUN"}
        case("observe.known-protection-pixels-ocr", protected_observation_check)

        def withheld_check():
            screen("protected-screenshot", f"{ROOT}/withheld.png", stdin=b'{"unknown":true,"targets":[],"values":[]}', accepted=(75,))
            execute("test", "!", "-e", f"{ROOT}/withheld.png")
            return {"unknownProtectionRejected": True, "noImagePublished": True}
        case("observe.unknown-protection-withheld", withheld_check)

        def editor_check():
            execute("touch", f"{ROOT}/document.txt")
            docker("exec", "-d", "-u", "slice", "-e", "DISPLAY=:99", name, "dbus-run-session", "mousepad", "--disable-server", f"{ROOT}/document.txt")
            window = wait(lambda: execute("xdotool", "search", "--onlyvisible", "--class", "Mousepad", accepted=(0, 1)).decode().strip())
            execute("xdotool", "windowactivate", "--sync", window.splitlines()[-1])
            time.sleep(.2)
            before = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            text = "Editor Grüße 世界\nsecond line\n"
            screen("computer-type-stdin", stdin=text.encode())
            screen("computer-key-hold-stdin", 750, stdin=b"shift+Left")
            screen("computer-key-stdin", 1, stdin=b"ctrl+c")
            assert screen("computer-clipboard-read").decode(), "editor hold did not create a physical selection"
            assert native_held() == {"keys": 0, "buttons": 0}, "editor hold left native input pressed"
            screen("computer-key-stdin", 1, stdin=b"ctrl+s")
            wait(lambda: execute("cat", f"{ROOT}/document.txt").decode() == text)
            screen("protected-screenshot", f"{ROOT}/editor.png", stdin=b'{"unknown":false,"targets":[],"values":[]}')
            after = json.loads(execute(PYTHON, "/opt/computer-source/slice-keyboard.py", "secret-target"))
            assert after == before, "non-browser editor focus/geometry changed"
            screen("computer-key-stdin", 1, stdin=b"ctrl+a")
            screen("computer-key-stdin", 1, stdin=b"ctrl+c")
            assert screen("computer-clipboard-read").decode() == text, "editor selection/copy differs from saved text"
            docker("cp", f"{name}:{ROOT}/editor.png", str(evidence / "editor.png"))
            return {"unicodeSave": True, "selectionCopy": True, "focusRetained": True}
        case("desktop.editor", editor_check)
        report["gatesNotEstablished"] = ["IME preedit/composition", "live public Room hold/takeover (source tests separate from this helper run)",
            "human clipboard through Web transport", "Room/host-browser surface identity", "official provider model-visible image",
            "Web/local TUI/remote TUI", "signed ordinary/managed comparison", "Vault secret restrictions"]
    except Exception as error:
        report["setupFailure"] = str(error)
    finally:
        stop.set()
        if watcher:
            watcher.join(12)
        if docker("ps", "-aq", "--filter", f"name=^/{name}$").strip():
            labels = docker("inspect", "--format", '{{index .Config.Labels "chariox.drill"}}', name).decode().strip()
            assert labels == "computer-input", "refuse cleanup of a foreign container"
            docker("rm", "-f", name)
        absent = not docker("ps", "-aq", "--filter", f"name=^/{name}$").strip()
        report["cleanup"] = {"containerAbsent": absent, "volumesCreated": 0, "publishedPorts": 0,
                             "runtimeState": "container writable layer removed", "imagesCreated": 0}
        report["resources"].append(resources())
        report["finishedAt"] = time.time()
        report["status"] = "PASS_HELPER_ONLY" if absent and "setupFailure" not in report and "resourceStop" not in report and all(
            c["status"] == "PASS" for c in report["cases"]) else "RED"
        (evidence / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"items": report["items"], "status": report["status"], "evidence": str(evidence),
                      "cases": [{"id": c["id"], "status": c["status"], "failure": c.get("failure")} for c in report["cases"]]}))
    return 0 if report["status"] == "PASS_HELPER_ONLY" else 1


if __name__ == "__main__":
    raise SystemExit(main())

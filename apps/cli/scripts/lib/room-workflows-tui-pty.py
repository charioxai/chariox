#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: drive the real terminal, retaining rendered evidence."""
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import termios
import time
import threading

import pyte
from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, sys.argv[1])
from owned_process_signals import OwnedProcesses

master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 160, 0, 0))
child = subprocess.Popen(sys.argv[2:], stdin=slave, stdout=slave, stderr=slave,
                         start_new_session=True)
os.close(slave)
owned = OwnedProcesses()
handle = owned.record(child.pid, watch=True, fresh_launch=True)
class TerminalScreen(pyte.Screen):
    def write_process_input(self, data):
        os.write(master, data.encode())

    def report_device_status(self, mode, private=False):
        if private and mode == 6:
            self.write_process_input(f"\x1b[?{self.cursor.y + 1};{self.cursor.x + 1}R")
        else:
            super().report_device_status(mode)


screen = TerminalScreen(160, 42)
stream = pyte.ByteStream(screen)
raw = bytearray()


def drain(duration=0.2):
    deadline = time.monotonic() + duration
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.05)[0]:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            raw.extend(data)
            stream.feed(data)


def capture(prefix, drain_seconds=0.2):
    drain(max(0, min(0.2, drain_seconds)))
    text = "\n".join(screen.display)
    Path(prefix + ".txt").write_text(text)
    Path(prefix + ".ansi.log").write_bytes(raw)
    fonts = ["/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
             "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]
    font_path = next((candidate for candidate in fonts if Path(candidate).is_file()), None)
    font = ImageFont.truetype(font_path, 14) if font_path else ImageFont.load_default()
    canvas = Image.new("RGB", (160 * 9, 42 * 18), "#111827")
    draw = ImageDraw.Draw(canvas)
    for y, line in enumerate(screen.display):
        draw.text((0, y * 18), line, font=font, fill="#e5e7eb")
    canvas.save(prefix + ".png")
    return {"text": text, "exitCode": child.poll()}


watch_stop = threading.Event()
watch_thread = None
watch_samples = []


def watch_process(pid, progress_path, start_ticks):
    if not isinstance(pid, int) or pid <= 1:
        raise ValueError("MP-11 invalid observed process")
    watch_samples.clear()
    watch_stop.clear()

    def observe():
        while not watch_stop.is_set():
            try:
                value = Path(progress_path).read_text()
            except FileNotFoundError:
                value = ""
            try:
                raw_stat = Path(f"/proc/{pid}/stat").read_text()
                fields = raw_stat[raw_stat.rfind(")") + 2:].split()
                alive = fields[0] != "Z" and fields[19] == start_ticks
            except FileNotFoundError:
                alive = False
            watch_samples.append({"at": time.time_ns() // 1_000_000,
                                  "value": value, "processAlive": alive})
            watch_stop.wait(0.02)

    thread = threading.Thread(target=observe, daemon=True)
    thread.start()
    return thread


try:
    for line in sys.stdin:
        request = json.loads(line)
        try:
            if request["action"] == "key":
                sent_at_ms = time.time_ns() // 1_000_000
                os.write(master, base64.b64decode(request["bytes"]))
                drain(max(0, min(0.2, request.get("drainSeconds", 0.2))))
                result = {"sentAtMs": sent_at_ms}
            elif request["action"] == "watch":
                if watch_thread is not None:
                    raise ValueError("MP-11 process observer already running")
                watch_thread = watch_process(request["pid"], request["progressPath"], request["startTicks"])
                result = {"watching": True}
            elif request["action"] == "finishWatch":
                watch_stop.set()
                if watch_thread is not None:
                    watch_thread.join(timeout=1)
                watch_thread = None
                result = {"samples": list(watch_samples)}
            elif request["action"] == "capture":
                result = capture(request["prefix"], request.get("drainSeconds", 0.2))
            elif request["action"] == "close":
                break
            else:
                raise ValueError("unknown MP-08 terminal operation")
            print(json.dumps({"id": request["id"], "result": result}), flush=True)
        except Exception as error:
            print(json.dumps({"id": request["id"], "error": str(error)}), flush=True)
finally:
    watch_stop.set()
    if child.poll() is None:
        owned.group(handle, 15)
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            owned.group(handle, 9)
            child.wait(timeout=3)
    owned.closed.set()
    os.close(master)

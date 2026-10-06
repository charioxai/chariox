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
screen = pyte.Screen(160, 42)
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


def capture(prefix):
    drain()
    text = "\n".join(screen.display)
    Path(prefix + ".txt").write_text(text)
    Path(prefix + ".ansi.log").write_bytes(raw)
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 14)
    canvas = Image.new("RGB", (160 * 9, 42 * 18), "#111827")
    draw = ImageDraw.Draw(canvas)
    for y, line in enumerate(screen.display):
        draw.text((0, y * 18), line, font=font, fill="#e5e7eb")
    canvas.save(prefix + ".png")
    return {"text": text, "exitCode": child.poll()}


try:
    for line in sys.stdin:
        request = json.loads(line)
        try:
            if request["action"] == "key":
                os.write(master, base64.b64decode(request["bytes"]))
                drain()
                result = {}
            elif request["action"] == "capture":
                result = capture(request["prefix"])
            elif request["action"] == "close":
                break
            else:
                raise ValueError("unknown MP-08 terminal operation")
            print(json.dumps({"id": request["id"], "result": result}), flush=True)
        except Exception as error:
            print(json.dumps({"id": request["id"], "error": str(error)}), flush=True)
finally:
    if child.poll() is None:
        owned.group(handle, 15)
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            owned.group(handle, 9)
            child.wait(timeout=3)
    owned.closed.set()
    os.close(master)

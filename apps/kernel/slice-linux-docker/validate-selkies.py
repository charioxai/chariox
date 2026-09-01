#!/usr/bin/env python3
"""Run inside the selkies-runtime image, with no host ports or privileges.

Exercises the packaged HTTP/WebSocket endpoints and real X11 capture. It does
not claim browser decoding, relay transport, or kernel input ownership coverage.
"""

import asyncio
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time

import aiohttp


BASE = "http://127.0.0.1:6080"


async def wait_ready(client, process):
    async with asyncio.timeout(20):
        while True:
            if process.poll() is not None:
                raise AssertionError(f"Selkies exited during startup: {process.returncode}")
            try:
                async with client.get(f"{BASE}/api/health") as response:
                    if response.status == 200 and await response.text() == "OK":
                        return
            except aiohttp.ClientError:
                pass
            await asyncio.sleep(0.1)


async def receive_keyframe(websocket):
    async with asyncio.timeout(15):
        while True:
            message = await websocket.receive()
            if message.type == aiohttp.WSMsgType.BINARY:
                packet = message.data
                if len(packet) <= 10 or packet[0] != 4:
                    continue
                # The published Selkies wire format uses network-order uint16s.
                _, keyframe, frame_id, stripe_y, width, height = struct.unpack(
                    ">BBHHHH", packet[:10]
                )
                await websocket.send_str(f"CLIENT_FRAME_ACK {frame_id}")
                if not keyframe:
                    continue
                assert stripe_y == 0, f"Unexpected striped encoder: {stripe_y}"
                assert (width, height) == (640, 480), (width, height)
                assert packet[10:].startswith((b"\0\0\0\1", b"\0\0\1")), "Not Annex-B H.264"
                return {"width": width, "height": height, "bytes": len(packet)}
            elif message.type in (
                aiohttp.WSMsgType.CLOSE,
                aiohttp.WSMsgType.CLOSED,
                aiohttp.WSMsgType.ERROR,
            ):
                raise AssertionError(f"WebSocket closed before video: {message}")


async def check_endpoints(process):
    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=5)) as client:
        await wait_ready(client, process)
        for route, content_type in (("/", "text/html"), ("/src/selkies-core.js", "javascript")):
            async with client.get(BASE + route) as response:
                assert response.status == 200, (route, response.status)
                assert content_type in response.headers.get("Content-Type", ""), route
                assert len(await response.read()) > 100, route
        async with client.get(BASE + "/manifest.json") as response:
            assert response.status == 200
            assert (await response.json())["start_url"] == "."
        async with client.ws_connect(
            BASE + "/api/websockets", origin=BASE, max_msg_size=4 * 1024 * 1024
        ) as websocket:
            await websocket.send_str("SETTINGS," + json.dumps({
                "displayId": "primary", "manual_resolution": True,
                "manual_width": 640, "manual_height": 480,
                "audioRedundancy": False,
            }))
            await websocket.send_str("START_VIDEO")
            frame = await receive_keyframe(websocket)
        return frame


def terminate(process):
    """TERM only our process group, then bounded forced cleanup on failure."""
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
        raise AssertionError(f"Process {process.pid} required forced shutdown")


def check_lifecycle(script, environment):
    """Public lifecycle commands, including idempotence and crash recovery."""
    def command(action, expected=0, extra=None):
        result = subprocess.run(
            [sys.executable, script, action], env={**environment, **(extra or {})},
            capture_output=True, text=True, timeout=30,
        )
        assert result.returncode == expected, (action, result.stdout, result.stderr)
        return json.loads(result.stdout)

    try:
        assert command("status", 1)["available"] is False
        starters = [subprocess.Popen([sys.executable, script, "start"], env=environment,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    for _ in range(2)]
        started = []
        for starter in starters:
            output, error = starter.communicate(timeout=30)
            assert starter.returncode == 0, error
            started.append(json.loads(output))
        first = started[0]
        assert started[1]["pid"] == first["pid"], "concurrent start duplicated the streamer"
        assert first["available"] is True
        assert first["kind"] == "selkies"
        assert command("start")["pid"] == first["pid"], "start duplicated the streamer"
        assert command("status")["pid"] == first["pid"]
        assert "token" not in json.dumps(first), "private credential leaked in status"
        # An unprovisioned viewer must not bypass kernel admission.
        async def refused_viewer():
            async with aiohttp.ClientSession() as client:
                try:
                    async with client.ws_connect(BASE + "/api/websockets", origin=BASE) as ws:
                        async with asyncio.timeout(3):
                            while True:
                                reply = await ws.receive()
                                assert reply.type != aiohttp.WSMsgType.BINARY, "anonymous viewer received video"
                                if reply.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED):
                                    return
                except aiohttp.WSServerHandshakeError as error:
                    assert error.status in (401, 403), error
        asyncio.run(refused_viewer())
        os.kill(first["pid"], signal.SIGKILL)
        for _ in range(50):
            dead = subprocess.run([sys.executable, script, "status"], env=environment,
                                  capture_output=True, timeout=5)
            if dead.returncode == 1:
                break
            time.sleep(0.05)
        else:
            raise AssertionError("crashed streamer still reported available")
        recovered = command("start")
        assert recovered["pid"] != first["pid"]
        os.kill(recovered["pid"], signal.SIGSTOP)
        assert command("stop", 1)["forced"] is True, "state-capture stop must report forced shutdown"
        command("start")
        assert command("stop")["forced"] is False
        assert command("stop")["stopped"] is True
        assert command("status", 1)["available"] is False
        print(json.dumps({"lifecycle": "pass", "restart_after_crash": "pass",
                          "anonymous_viewer": "denied", "idempotent_start_stop": "pass"}))
    finally:
        subprocess.run([sys.executable, script, "stop"], env=environment,
                       capture_output=True, timeout=30)


def main():
    with tempfile.TemporaryDirectory(prefix="chariox-selkies-drill-") as scratch:
        environment = {**os.environ, "DISPLAY": ":91", "HOME": scratch,
                       "XDG_RUNTIME_DIR": scratch, "OMP_NUM_THREADS": "1"}
        processes = []
        try:
            with open(Path(scratch) / "xvfb.log", "w+") as xlog, open(
                Path(scratch) / "selkies.log", "w+"
            ) as slog:
                xvfb = subprocess.Popen(
                    ["Xvfb", ":91", "-screen", "0", "640x480x24", "-nolisten", "tcp", "-ac"],
                    env=environment, stdout=xlog, stderr=subprocess.STDOUT, start_new_session=True,
                )
                processes.append(xvfb)
                for attempt in range(100):
                    probe = subprocess.run(["xdpyinfo"], env=environment, capture_output=True)
                    if probe.returncode == 0:
                        break
                    if xvfb.poll() is not None:
                        raise AssertionError("Xvfb exited during startup")
                    time.sleep(0.05)
                else:
                    raise AssertionError("X11 display did not become ready")
                subprocess.run(["xsetroot", "-solid", "#2364aa"], env=environment, check=True)
                server = subprocess.Popen([
                    os.environ.get("CHARIOX_SLICE_SELKIES_BIN", "/opt/chariox-selkies/bin/selkies"),
                    "--addr=127.0.0.1", "--port=6080", "--mode=websockets",
                    "--encoder=h264enc", "--use-cpu=true", "--framerate=10",
                    "--enable-https=false", "--enable-basic-auth=false",
                    "--audio-enabled=false", "--gamepad-enabled=false",
                    "--command-enabled=false", "--file-transfers=none",
                    "--webcam-enabled=false", "--microphone-enabled=false",
                    "--enable-resize=false", "--enable-clipboard=false",
                ], env=environment, stdout=slog, stderr=subprocess.STDOUT, start_new_session=True)
                processes.append(server)
                try:
                    frame = asyncio.run(check_endpoints(server))
                    terminate(server)
                    assert server.returncode in (0, -signal.SIGTERM), server.returncode
                    with socket.socket() as probe:
                        assert probe.connect_ex(("127.0.0.1", 6080)) != 0, "Listener survived shutdown"
                    lifecycle = os.environ.get("CHARIOX_TEST_SELKIES_LIFECYCLE")
                    if lifecycle:
                        check_lifecycle(lifecycle, environment)
                    terminate(xvfb)
                    assert not Path("/tmp/.X11-unix/X91").exists(), "X11 socket survived shutdown"
                    print(json.dumps({"result": "pass", "software_h264": frame,
                                      "http_assets": "pass", "graceful_shutdown": "pass"}))
                except BaseException:
                    slog.flush()
                    print((Path(scratch) / "selkies.log").read_text()[-12000:], flush=True)
                    raise
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    terminate(process)


if __name__ == "__main__":
    main()

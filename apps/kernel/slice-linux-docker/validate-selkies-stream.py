#!/usr/bin/env python3
"""Private stream CLI acceptance against the packaged, real Selkies runtime."""

import asyncio
import base64
import json
import os
from pathlib import Path
import random
import signal
import struct
import sys
import tempfile


ROOT = Path(os.environ.get("CHARIOX_TEST_SELKIES_ROOT", Path(__file__).resolve().parent / "docker"))


async def lifecycle(environment, action):
    process = await asyncio.create_subprocess_exec(
        sys.executable, str(ROOT / "slice-selkies.py"), action, env=environment,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    output, error = await asyncio.wait_for(process.communicate(), 30)
    assert process.returncode == 0, (action, error.decode())
    return json.loads(output)


async def start_stream(environment, lease_ms=60000, read_limit=6 * 1024 * 1024):
    return await asyncio.create_subprocess_exec(
        sys.executable, str(ROOT / "slice-selkies-stream.py"), "--lease-ms", str(lease_ms),
        env=environment, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE, limit=read_limit,
    )


async def message(process):
    line = await asyncio.wait_for(process.stdout.readline(), 15)
    if not line:
        _, error = await asyncio.wait_for(process.communicate(), 5)
        raise AssertionError(f"private stream closed before expected frame: {error.decode()}")
    return json.loads(line)


async def command(process, payload):
    process.stdin.write(json.dumps(payload).encode() + b"\n")
    await process.stdin.drain()


async def keyframe(process):
    async with asyncio.timeout(15):
        while True:
            event = await message(process)
            if event["kind"] != "binary":
                continue
            packet = base64.b64decode(event["data_base64"], validate=True)
            assert packet[0] == 4 and len(packet) > 10
            _, is_keyframe, frame_id, stripe_y, width, height = struct.unpack(
                ">BBHHHH", packet[:10]
            )
            await command(process, {"kind": "control", "text": f"CLIENT_FRAME_ACK {frame_id}"})
            if is_keyframe:
                assert (stripe_y, width, height) == (0, 640, 480), (stripe_y, width, height)
                assert packet[10:].startswith((b"\0\0\0\1", b"\0\0\1"))
                return len(packet)


async def text_event(process, text):
    async with asyncio.timeout(5):
        while True:
            event = await message(process)
            if event == {"kind": "text", "text": text}:
                return


async def assert_ready(process):
    assert await message(process) == {"kind": "ready", "protocol": "selkies-stdio-v1", "read_only": True}


async def close_stream(process):
    await command(process, {"kind": "close"})
    output, error = await asyncio.wait_for(process.communicate(), 5)
    assert process.returncode == 0, error.decode()
    assert any(json.loads(line) == {"kind": "closed", "reason": "closed"} for line in output.splitlines())


async def stop_process(process):
    if process.returncode is None:
        process.terminate()
        try:
            await asyncio.wait_for(process.communicate(), 5)
        except asyncio.TimeoutError:
            process.kill()
            await asyncio.wait_for(process.communicate(), 5)


async def main():
    processes = []
    with tempfile.TemporaryDirectory(prefix="chariox-private-stream-") as scratch:
        environment = {**os.environ, "DISPLAY": ":93", "HOME": scratch,
                       "XDG_RUNTIME_DIR": scratch, "OMP_NUM_THREADS": "1"}
        try:
            xvfb = await asyncio.create_subprocess_exec(
                "Xvfb", ":93", "-screen", "0", "640x480x24", "-nolisten", "tcp", "-ac",
                env=environment, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL,
            )
            processes.append(xvfb)
            for _ in range(100):
                probe = await asyncio.create_subprocess_exec(
                    "xdpyinfo", env=environment, stdout=asyncio.subprocess.DEVNULL,
                    stderr=asyncio.subprocess.DEVNULL,
                )
                if await probe.wait() == 0:
                    break
                await asyncio.sleep(0.05)
            else:
                raise AssertionError("Xvfb did not start")
            await lifecycle(environment, "start")
            first = await start_stream(environment)
            processes.append(first)
            await assert_ready(first)
            frame_bytes = await keyframe(first)
            second = await start_stream(environment)
            processes.append(second)
            await assert_ready(second)
            await keyframe(second)
            await command(first, {"kind": "control", "text": "STOP_VIDEO"})
            await text_event(first, "VIDEO_STOPPED")
            await close_stream(first)
            await command(second, {"kind": "control", "text": "STOP_VIDEO"})
            await text_event(second, "VIDEO_STOPPED")
            await command(second, {"kind": "control", "text": "START_VIDEO"})
            await keyframe(second)
            await close_stream(second)

            expired = await start_stream(environment, lease_ms=300)
            processes.append(expired)
            await assert_ready(expired)
            output, error = await asyncio.wait_for(expired.communicate(), 5)
            assert expired.returncode == 0, error.decode()
            assert any(json.loads(line) == {"kind": "closed", "reason": "lease_expired"} for line in output.splitlines())

            renewed = await start_stream(environment, lease_ms=700)
            processes.append(renewed)
            await assert_ready(renewed)
            for _ in range(3):
                await asyncio.sleep(0.4)
                await command(renewed, {"kind": "renew"})
            await command(renewed, {"kind": "control", "text": "STOP_VIDEO"})
            await text_event(renewed, "VIDEO_STOPPED")
            await close_stream(renewed)

            for forbidden in ("m,100,100,0", "kd,65", "SETTINGS,{}", "CLIPBOARD,DO_NOT_LOG_SECRET", "CLIENT_FRAME_ACK 65536"):
                denied = await start_stream(environment)
                processes.append(denied)
                await assert_ready(denied)
                await command(denied, {"kind": "control", "text": forbidden})
                output, error = await asyncio.wait_for(denied.communicate(), 5)
                assert denied.returncode == 1, "unsafe viewer control was accepted"
                assert b"DO_NOT_LOG_SECRET" not in output + error, "rejected content leaked"

            for _ in range(9):
                crashed = await start_stream(environment)
                processes.append(crashed)
                await assert_ready(crashed)
                crashed.kill()
                await asyncio.wait_for(crashed.communicate(), 5)
                assert crashed.returncode == -signal.SIGKILL

            old = await start_stream(environment)
            processes.append(old)
            await assert_ready(old)
            os.kill(old.pid, signal.SIGSTOP)
            await lifecycle(environment, "stop")
            await lifecycle(environment, "start")
            current = await start_stream(environment)
            processes.append(current)
            await assert_ready(current)
            os.kill(old.pid, signal.SIGCONT)
            await asyncio.wait_for(old.communicate(), 5)
            assert old.returncode == 1, "disconnected old streamer should fail"
            await keyframe(current)
            await command(current, {"kind": "control", "text": "STOP_VIDEO"})
            await text_event(current, "VIDEO_STOPPED")
            await close_stream(current)

            # Make real, poorly compressible display changes while the parent
            # stops reading. A small reader limit prevents asyncio from hiding
            # the stalled consumer in its own multi-megabyte input buffer.
            stalled = await start_stream(environment, read_limit=1024)
            processes.append(stalled)
            bitmap = Path(scratch) / "noise.xbm"
            rng = random.Random(42)
            for _ in range(12):
                # XReadBitmapFile accepts short C initializer lines, not one
                # 192KB line. Keep the fixture in the normal XBM format.
                values = [f"0x{rng.getrandbits(8):02x}" for _ in range(640 * 480 // 8)]
                rows = [",".join(values[index:index + 16]) for index in range(0, len(values), 16)]
                bitmap.write_text("#define noise_width 640\n#define noise_height 480\nstatic unsigned char noise_bits[] = {\n" + ",\n".join(rows) + "};\n")
                paint = await asyncio.create_subprocess_exec(
                    "xsetroot", "-bitmap", str(bitmap), env=environment,
                    stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE,
                )
                _, error = await paint.communicate()
                assert paint.returncode == 0, error.decode()
                await asyncio.sleep(0.1)
            async with asyncio.timeout(6):
                while stalled.returncode is None:
                    await asyncio.sleep(0.1)
            await stalled.communicate()
            assert stalled.returncode == 1, "stalled output should fail within its bounded write timeout"

            closing = await start_stream(environment, read_limit=1024)
            processes.append(closing)
            await assert_ready(closing)
            await asyncio.sleep(0.3)
            await command(closing, {"kind": "close"})
            await asyncio.sleep(0.05)
            output, error = await asyncio.wait_for(closing.communicate(), 5)
            assert closing.returncode == 0, error.decode()
            records = [json.loads(line) for line in output.splitlines()]
            assert records[-1] == {"kind": "closed", "reason": "closed"}
            assert any(record["kind"] == "binary" for record in records)
            print(json.dumps({"private_stream": "pass", "software_h264_bytes": frame_bytes,
                              "simultaneous_viewers": "pass", "lease_expiry_renewal": "pass",
                              "unsafe_input": "rejected", "stale_token_pruning": "pass",
                              "restart_generation_isolation": "pass", "stalled_reader": "bounded_close",
                              "close_during_frame": "complete_records"}))
        finally:
            for process in reversed(processes[1:]):
                await stop_process(process)
            await lifecycle(environment, "stop")
            for process in processes[:1]:
                await stop_process(process)
            assert not Path("/tmp/.X11-unix/X93").exists(), "X11 socket survived cleanup"


if __name__ == "__main__":
    asyncio.run(main())

#!/usr/bin/env python3
"""Kernel-owned private pipe to Selkies; stdout contains video, never tokens."""

import argparse
import asyncio
import base64
import json
import os
import re
import signal
import sys

import aiohttp

from selkies_viewers import PrivateStreamError, ViewerAccess


MAX_MESSAGE_BYTES = 4 * 1024 * 1024


async def emit(value):
    data = memoryview(json.dumps(value, separators=(",", ":")).encode() + b"\n")
    loop = asyncio.get_running_loop()
    descriptor = sys.stdout.fileno()
    async with asyncio.timeout(2):
        while data:
            try:
                data = data[os.write(descriptor, data):]
            except BlockingIOError:
                ready = loop.create_future()
                loop.add_writer(descriptor, lambda: ready.done() or ready.set_result(None))
                try:
                    await ready
                finally:
                    loop.remove_writer(descriptor)


async def connect_viewer(client, access):
    # Selkies has a 500ms per-IP reconnect debounce, including separate viewers
    # of this private loopback endpoint. Retry only that explicit refusal.
    for attempt in range(4):
        websocket = await client.ws_connect(
            access.origin + "/api/websockets", params={"token": access.token},
            origin=access.origin, max_msg_size=MAX_MESSAGE_BYTES, heartbeat=15,
        )
        authenticated = False
        try:
            async with asyncio.timeout(5):
                while True:
                    message = await websocket.receive()
                    if message.type == aiohttp.WSMsgType.TEXT:
                        if message.data.startswith("AUTH_SUCCESS,"):
                            authenticated = json.loads(message.data.split(",", 1)[1]).get("role") == "viewer"
                            if not authenticated:
                                raise PrivateStreamError("private stream was not read-only")
                        elif message.data == "MK_ACCESS,0" and authenticated:
                            return websocket
                        elif message.data == "MK_ACCESS,1":
                            raise PrivateStreamError("private stream received input authority")
                    elif message.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR):
                        break
            retry = websocket.close_code == 4029 and attempt < 3
        except BaseException:
            await websocket.close()
            raise
        await websocket.close()
        if not retry:
            raise PrivateStreamError("private viewer handshake failed")
        await asyncio.sleep(0.55)
    raise PrivateStreamError("private viewer handshake failed")


async def stream(access, lease_ms):
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader(limit=8192)
    transport, _ = await loop.connect_read_pipe(lambda: asyncio.StreamReaderProtocol(reader), sys.stdin.buffer)
    deadline = loop.time() + lease_ms / 1000
    timeout = aiohttp.ClientTimeout(total=None, sock_connect=3)
    try:
        async with aiohttp.ClientSession(timeout=timeout, trust_env=False) as client:
            websocket = await connect_viewer(client, access)
            async with websocket:
                await websocket.send_str('SETTINGS,{"displayId":"primary","audioRedundancy":false}')
                await websocket.send_str("START_VIDEO")
                deadline = loop.time() + lease_ms / 1000
                await emit({"kind": "ready", "protocol": "selkies-stdio-v1", "read_only": True})
                output = asyncio.Queue(maxsize=1)

                async def write_output():
                    # One writer owns every complete record. Cancellation of
                    # frame reception must not splice a close into video bytes.
                    while True:
                        value = await output.get()
                        await emit(value)
                        if value["kind"] == "closed":
                            return

                async def controls():
                    nonlocal deadline
                    while line := await reader.readline():
                        value = json.loads(line)
                        if not isinstance(value, dict):
                            raise PrivateStreamError("unsupported private viewer control")
                        if value == {"kind": "close"}:
                            return "closed"
                        if value == {"kind": "renew"}:
                            deadline = loop.time() + lease_ms / 1000
                            continue
                        text = value.get("text")
                        if value.get("kind") != "control" or set(value) != {"kind", "text"} or not isinstance(text, str):
                            raise PrivateStreamError("unsupported private viewer control")
                        ack = re.fullmatch(r"CLIENT_FRAME_ACK ([0-9]{1,5})", text)
                        if text not in ("START_VIDEO", "STOP_VIDEO", "REQUEST_KEYFRAME") and not (ack and int(ack[1]) <= 65535):
                            raise PrivateStreamError("unsupported private viewer control")
                        await asyncio.wait_for(websocket.send_str(text), 2)
                    return "closed"

                async def frames():
                    async for message in websocket:
                        if message.type == aiohttp.WSMsgType.BINARY:
                            # Only video leaves the slice. Clipboard, audio, and
                            # device messages are not part of this viewer path.
                            if len(message.data) > 10 and message.data[0] == 4:
                                await output.put({"kind": "binary", "data_base64": base64.b64encode(message.data).decode()})
                        elif message.type == aiohttp.WSMsgType.TEXT:
                            if message.data in ("PIPELINE_RESETTING primary", "VIDEO_STARTED", "VIDEO_STOPPED"):
                                await output.put({"kind": "text", "text": message.data})
                        elif message.type == aiohttp.WSMsgType.ERROR:
                            raise PrivateStreamError("private stream disconnected")
                    raise PrivateStreamError("private stream disconnected")

                async def expiry():
                    while loop.time() < deadline:
                        await asyncio.sleep(min(0.1, deadline - loop.time()))
                    return "lease_expired"

                writer = asyncio.create_task(write_output())
                producers = [asyncio.create_task(job()) for job in (controls, frames, expiry)]
                tasks = [writer, *producers]
                try:
                    done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
                    for task in done:
                        if task.exception() is not None:
                            raise task.exception()
                    reason = next(iter(done)).result()
                    for task in producers:
                        task.cancel()
                    await asyncio.gather(*producers, return_exceptions=True)
                    async with asyncio.timeout(5):
                        await output.put({"kind": "closed", "reason": reason})
                        await writer
                finally:
                    for task in tasks:
                        task.cancel()
                    await asyncio.gather(*tasks, return_exceptions=True)
    finally:
        transport.close()


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lease-ms", type=int, default=60000)
    lease_ms = parser.parse_args().lease_ms
    if not 200 <= lease_ms <= 60000:
        raise ValueError("invalid private viewer lease duration")
    os.set_blocking(sys.stdout.fileno(), False)
    current = asyncio.current_task()
    loop = asyncio.get_running_loop()
    for signum in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(signum, current.cancel)
    with ViewerAccess() as access:
        await stream(access, lease_ms)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except PrivateStreamError as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except (Exception, asyncio.CancelledError) as error:
        # aiohttp exceptions can contain the private token URL. Never print it.
        print(f"private Selkies stream ended or failed ({type(error).__name__})", file=sys.stderr)
        sys.exit(1)

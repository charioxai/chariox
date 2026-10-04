"""Apply kernel-owned geometry. Viewer sockets never supply these dimensions."""
import asyncio
import json
import os
import re
import secrets
import struct
import subprocess
import sys
import time
import urllib.request



_deadline = None


class DisplayError(RuntimeError):
    pass


def dimensions(width, height):
    # The pinned H264 path encodes even dimensions. Bound framebuffer allocation.
    if any(type(value) is not int or value < 64 or value > 4096 or value % 2
           for value in (width, height)):
        raise DisplayError("unsupported canonical display dimensions")
    return width, height


def remaining():
    seconds = 30 if _deadline is None else _deadline - time.monotonic()
    if seconds <= 0:
        raise DisplayError("canonical display deadline expired")
    return seconds


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=min(1, remaining()), check=False)
    if result.returncode:
        raise DisplayError("canonical display command failed")
    return result.stdout


def geometry():
    match = re.search(r"dimensions:\s+(\d+)x(\d+) pixels", run("xdpyinfo"))
    if not match:
        raise DisplayError("canonical display geometry unavailable")
    return tuple(map(int, match.groups()))


def resize(width, height):
    if os.environ.get("CHARIOX_SLICE_DISPLAY_SERVER") == "Xvfb":
        if geometry() != (width, height):
            raise DisplayError("physical resize requires the managed Xorg image")
        return
    dimensions(width, height)
    outputs = re.findall(r"^(\S+) connected", run("xrandr", "--query"), re.M)
    if len(outputs) != 1:
        raise DisplayError("canonical display requires one physical output")
    output = outputs[0]
    mode = f"chariox-{width}x{height}"
    query = run("xrandr", "--query")
    selected = False
    modes = set()
    for line in query.splitlines():
        if line and not line[0].isspace():
            selected = line.split()[0] == output
        elif selected and line.split():
            modes.add(line.split()[0])
    if mode not in modes:
        timing = run("cvt", "-r", str(width), str(height), "60")
        line = next((line for line in timing.splitlines() if line.startswith("Modeline")), None)
        if line is None:
            raise DisplayError("canonical display timing unavailable")
        # CVT rounds horizontal active pixels to a multiple of eight. The dummy
        # driver supports the exact even framebuffer, within the same blanking.
        fields = line.split()[2:]
        fields[1] = str(width)
        run("xrandr", "--newmode", mode, *fields)
        run("xrandr", "--addmode", output, mode)
    run("xrandr", "--output", output, "--mode", mode)
    if geometry() != (width, height):
        raise DisplayError("canonical physical display dimensions disagree")


async def refresh_stream(record, width, height):
    import aiohttp
    from selkies_viewers import lifecycle, NoRedirect
    # An internal controller cannot send mouse/keyboard input. It only causes
    # the pinned streamer to re-read the physical X root with resize disabled.
    token = secrets.token_urlsafe(32)
    viewers = {key: {"role": "viewer", "mk_control": False}
               for key, owner in record.get("viewers", {}).items()
               if lifecycle.owned_process(owner) is not None}
    viewers[token] = {"role": "controller", "mk_control": False}
    # The pinned server grants controllers input when there is no MK owner,
    # ignoring mk_control:false in that fallback. An unconnected random owner
    # makes the refresh controller explicitly input-less. Neither token leaves
    # this process, and the exact viewer-only table is restored under the lock.
    viewers[secrets.token_urlsafe(32)] = {"role": "viewer", "mk_control": True}
    endpoint = lifecycle.endpoint(record)
    request = urllib.request.Request(endpoint + "/api/tokens", method="POST",
        data=json.dumps(viewers).encode(), headers={
            "Authorization": "Bearer " + record["master_token"], "Content-Type": "application/json"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=min(2, remaining())) as response:
        if response.status != 200:
            raise DisplayError("canonical capture registration failed")
    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=None, sock_connect=min(2, remaining())), trust_env=False) as http:
        async def connect(role):
            for attempt in range(4):
                socket = await http.ws_connect(endpoint + "/api/websockets", params={"token": token}, origin=endpoint)
                authenticated = False
                while True:
                    message = await socket.receive()
                    if message.type == aiohttp.WSMsgType.TEXT:
                        if message.data.startswith("AUTH_SUCCESS,"):
                            authenticated = json.loads(message.data.split(",", 1)[1]).get("role") == role
                            if not authenticated:
                                await socket.close()
                                raise DisplayError("canonical capture role disagrees")
                        elif message.data == "MK_ACCESS,0" and authenticated:
                            return socket
                        elif message.data == "MK_ACCESS,1":
                            await socket.close()
                            raise DisplayError("canonical capture received input authority")
                    elif message.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR):
                        break
                retry = socket.close_code == 4029 and attempt < 3
                await socket.close()
                if not retry:
                    raise DisplayError("canonical capture authentication failed")
                await asyncio.sleep(0.55)
            raise DisplayError("canonical capture authentication failed")

        async with asyncio.timeout(min(5, remaining())):
            async with await connect("controller") as socket:
                await socket.send_str('SETTINGS,{"displayId":"primary","audioRedundancy":false}')
                while True:
                    message = await socket.receive()
                    if message.type == aiohttp.WSMsgType.TEXT and message.data.startswith("{"):
                        realized = json.loads(message.data)
                        if realized.get("type") == "stream_resolution" and (realized.get("width"), realized.get("height")) == (width, height):
                            break
                    elif message.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR):
                        raise DisplayError("canonical capture closed before geometry verification")

            # The pinned streamer's internal RandR path may round CVT width
            # to eight pixels while its capture dimensions remain exact. Put
            # the kernel's exact mode back before verifying any encoded frame.
            if geometry() != (width, height):
                resize(width, height)

            # Verify encoded bytes through a read-only socket, exactly as Room
            # viewers do. The temporary controller never requests video/input.
            viewers[token] = {"role": "viewer", "mk_control": False}
            request.data = json.dumps(viewers).encode()
            with opener.open(request, timeout=min(2, remaining())) as response:
                if response.status != 200:
                    raise DisplayError("canonical verification viewer registration failed")
            async with await connect("viewer") as socket:
                await socket.send_str('SETTINGS,{"displayId":"primary","audioRedundancy":false}')
                await socket.send_str("START_VIDEO")
                while True:
                    message = await socket.receive()
                    if message.type == aiohttp.WSMsgType.BINARY:
                        if len(message.data) < 10:
                            raise DisplayError("canonical capture frame invalid")
                        kind, keyframe, frame, _, actual_width, actual_height = struct.unpack(">BBHHHH", message.data[:10])
                        if kind != 4:
                            continue
                        await socket.send_str(f"CLIENT_FRAME_ACK {frame}")
                        if keyframe and (actual_width, actual_height) == (width, height):
                            return
                    elif message.type in (aiohttp.WSMsgType.CLOSE, aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR):
                        raise DisplayError("canonical capture closed before verification")


def apply_vnc(width, height):
    from canonical_vnc import VncGeometry
    global _deadline
    started = time.monotonic()
    _deadline = started + 17
    observer = None
    try:
        previous = geometry()
        observer = VncGeometry(int(os.environ.get("CHARIOX_SLICE_VNC_PORT", "5900")), remaining)
        if observer.size != previous:
            raise DisplayError("canonical framebuffer and physical display disagree")
        if previous == (width, height):
            return
        observer.prime()
        try:
            observer.request_update()
            resize(width, height)
            observer.expect_resize(width, height)
            if geometry() != (width, height):
                raise DisplayError("canonical physical display changed during framebuffer verification")
        except BaseException as error:
            _deadline = started + 28
            try:
                observer.request_update()
                resize(*previous)
                observer.expect_resize(*previous)
            except BaseException:
                raise DisplayError("canonical framebuffer rollback failed") from None
            raise DisplayError("canonical framebuffer apply failed: " + type(error).__name__) from None
    finally:
        if observer is not None:
            observer.close()
        _deadline = None


def apply(width, height):
    global _deadline
    dimensions(width, height)
    if not os.environ.get("DISPLAY"):
        raise DisplayError("canonical physical display unavailable")
    if os.environ.get("CHARIOX_SLICE_DISPLAY_SERVER") == "Xvfb":
        resize(width, height)  # Refuse unsupported changes before capture mutation.
    if os.environ.get("CHARIOX_SLICE_VIEWER_BACKEND", "selkies") == "novnc":
        return apply_vnc(width, height)
    from selkies_viewers import locked_state, publish, lifecycle
    started = time.monotonic()
    # Total30s includes lock wait, apply, rollback and token-table restoration.
    # Reserve11s for rollback and2s for the final viewer-only API publication.
    _deadline = started + 17
    try:
        with locked_state(timeout=min(3, remaining())) as (directory, record):
            if record is None or lifecycle.owned_process(record) is None or not lifecycle.healthy(record):
                raise DisplayError("canonical streamer unavailable")
            previous = geometry()
            verified = {"width": width, "height": height}
            if previous == (width, height) and record.get("canonical_display") == verified:
                return
            record.pop("canonical_display", None)
            try:
                resize(width, height)
                asyncio.run(refresh_stream(record, width, height))
                # Starting the verification viewer can run the pinned
                # streamer's RandR setup again and round CVT width to eight
                # pixels. Encoded readback above proved the exact capture;
                # restore the kernel's exact physical mode after that setup.
                if geometry() != (width, height):
                    resize(width, height)
                if geometry() != (width, height):
                    raise DisplayError("canonical physical display changed during capture verification")
                record["canonical_display"] = verified
            except BaseException as error:
                _deadline = started + 28
                # Never acknowledge an unverified geometry. Restore the same live
                # display/streamer; restarting Chrome would discard its tabs.
                try:
                    resize(*previous)
                    asyncio.run(refresh_stream(record, *previous))
                except BaseException as rollback_error:
                    primary = str(error) if isinstance(error, DisplayError) else type(error).__name__
                    rollback = str(rollback_error) if isinstance(rollback_error, DisplayError) else type(rollback_error).__name__
                    raise DisplayError("canonical display rollback failed: " + primary + "; " + rollback) from None
                diagnostic = str(error) if isinstance(error, DisplayError) else type(error).__name__
                raise DisplayError("canonical display apply failed: " + diagnostic) from None
            finally:
                # The lock excludes viewer registration, so restore the exact live
                # viewer-only table without overwriting a concurrent lease.
                _deadline = started + 30
                try:
                    publish(directory, record)
                finally:
                    _deadline = None
    finally:
        _deadline = None


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            raise DisplayError("canonical display requires width and height")
        apply(int(sys.argv[1]), int(sys.argv[2]))
        print(json.dumps({"width": int(sys.argv[1]), "height": int(sys.argv[2])}))
    except BaseException as error:
        # Never expose private token tables, backend payloads or HTTP errors.
        diagnostic = str(error) if isinstance(error, DisplayError) else "canonical physical display apply failed"
        print(diagnostic, file=sys.stderr)
        sys.exit(1)

"""Boundary regressions. Live H264 readback is checked separately."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch, AsyncMock
import aiohttp
import selkies_viewers as viewers

spec = importlib.util.spec_from_file_location("canonical_display", Path(__file__).with_name("canonical-display.py"))
display = importlib.util.module_from_spec(spec)
spec.loader.exec_module(display)


class DisplayBoundaryTests(unittest.TestCase):
    def test_exact_supported_geometry_is_bounded_and_h264_even(self):
        for value in [(1024, 768), (390, 844), (4096, 4096)]:
            self.assertEqual(display.dimensions(*value), value)
        for value in [(0, 768), (4098, 768), (391, 844), (True, 768), ("1024", 768)]:
            with self.assertRaises(display.DisplayError):
                display.dimensions(*value)

    def test_old_xvfb_allows_current_physical_size_without_mutation(self):
        with patch.dict(display.os.environ, {"CHARIOX_SLICE_DISPLAY_SERVER": "Xvfb"}), patch.object(display, "geometry", return_value=(1280, 800)), patch.object(display, "run") as run:
            display.resize(1280, 800)
            run.assert_not_called()
            with self.assertRaisesRegex(display.DisplayError, "requires the managed Xorg"):
                display.resize(1024, 768)
            run.assert_not_called()

    def test_physical_readback_refuses_rounding(self):
        with patch.object(display, "geometry", return_value=(392, 844)), patch.object(display, "run") as run:
            run.side_effect = ["DUMMY0 connected\n", "DUMMY0 connected\n   chariox-390x844 60.00\n", ""]
            with self.assertRaisesRegex(display.DisplayError, "dimensions disagree"):
                display.resize(390, 844)
            self.assertEqual(run.call_args.args, ("xrandr", "--output", "DUMMY0", "--mode", "chariox-390x844"))

    def test_cvt_rounding_does_not_replace_requested_width(self):
        with patch.object(display, "geometry", return_value=(390, 844)), patch.object(display, "run") as run:
            run.side_effect = ["DUMMY0 connected\n", "", 'Modeline "392x844R" 25.00 392 440 472 552 844 847 857 875 +hsync -vsync', "", "", ""]
            display.resize(390, 844)
            self.assertEqual(run.call_args_list[3].args[4], "390")

    def test_mode_height_prefix_does_not_skip_creation(self):
        query = "DUMMY0 connected primary 1280x800+0+0\n   chariox-1280x800 60.00*\n"
        with patch.object(display, "geometry", return_value=(1280, 80)), patch.object(display, "run") as run:
            run.side_effect = [query, query, 'Modeline "1280x80R" 6.00 1280 1328 1360 1440 80 83 93 100 +hsync -vsync', "", "", ""]
            display.resize(1280, 80)
            self.assertEqual(run.call_args_list[3].args[:3], ("xrandr", "--newmode", "chariox-1280x80"))
            self.assertEqual(run.call_args_list[4].args, ("xrandr", "--addmode", "DUMMY0", "chariox-1280x80"))

    def test_slow_command_uses_remaining_deadline(self):
        import time
        import subprocess
        import sys
        display._deadline = time.monotonic() + 0.03
        started = time.monotonic()
        try:
            with self.assertRaises(subprocess.TimeoutExpired):
                display.run(sys.executable, "-c", "import time; time.sleep(1)")
            self.assertLess(time.monotonic() - started, 0.5)
        finally:
            display._deadline = None

    def test_slow_capture_obeys_the_remaining_phase_deadline(self):
        import asyncio
        import time
        from types import SimpleNamespace
        text = aiohttp.WSMsgType.TEXT
        class Socket:
            def __init__(self, messages): self.messages = messages
            async def __aenter__(self): return self
            async def __aexit__(self, *_): pass
            async def send_str(self, _): pass
            async def close(self): pass
            async def receive(self):
                if self.messages:
                    return SimpleNamespace(type=text, data=self.messages.pop(0))
                await asyncio.Future()
        class Client:
            def __init__(self):
                self.sockets = [Socket(['AUTH_SUCCESS,{"role":"controller"}', 'MK_ACCESS,0',
                    '{"type":"stream_resolution","width":1024,"height":768}']),
                    Socket(['AUTH_SUCCESS,{"role":"viewer"}', 'MK_ACCESS,0'])]
            async def __aenter__(self): return self
            async def __aexit__(self, *_): pass
            async def ws_connect(self, *args, **kwargs): return self.sockets.pop(0)
        class Response:
            status = 200
            def __enter__(self): return self
            def __exit__(self, *_): pass
        display._deadline = time.monotonic() + 0.05
        started = time.monotonic()
        try:
            with patch.object(aiohttp, "ClientSession", return_value=Client()), \
                 patch.object(display.urllib.request, "build_opener") as opener, \
                 patch.object(viewers.lifecycle, "endpoint", return_value="http://127.0.0.1:6080"), \
                 patch.object(display, "geometry", return_value=(1024, 768)):
                opener.return_value.open.return_value = Response()
                with self.assertRaises(TimeoutError):
                    asyncio.run(display.refresh_stream({"master_token": "synthetic"}, 1024, 768))
            self.assertLess(time.monotonic() - started, 0.5)
        finally:
            display._deadline = None

    def test_expired_apply_keeps_reserved_rollback_and_table_cleanup(self):
        class Lock:
            def __enter__(self): return "directory", {"master_token": "synthetic"}
            def __exit__(self, *_): pass
        deadlines = []
        refresh = AsyncMock(side_effect=[display.DisplayError("capture deadline"), None])
        with patch.dict(display.os.environ, {"DISPLAY": ":93"}), patch.object(display.time, "monotonic", return_value=100), \
             patch.object(viewers, "locked_state", return_value=Lock()), patch.object(viewers.lifecycle, "owned_process", return_value=object()), \
             patch.object(viewers.lifecycle, "healthy", return_value=True), patch.object(display, "geometry", return_value=(1280, 800)), \
             patch.object(display, "resize", side_effect=lambda *args: deadlines.append(display._deadline)), \
             patch.object(display, "refresh_stream", refresh), patch.object(viewers, "publish", side_effect=lambda *args: deadlines.append(display._deadline)):
            with self.assertRaises(display.DisplayError):
                display.apply(1024, 768)
        self.assertEqual(deadlines, [117, 128, 130])
        self.assertIsNone(display._deadline)

    def test_unverified_capture_rolls_back_before_refusal(self):
        class Lock:
            def __enter__(self): return "directory", {"master_token": "synthetic"}
            def __exit__(self, *_): pass
        refresh = AsyncMock(side_effect=[display.DisplayError("capture mismatch"), None])
        with patch.dict(display.os.environ, {"DISPLAY": ":93"}), patch.object(viewers, "locked_state", return_value=Lock()), \
             patch.object(viewers.lifecycle, "owned_process", return_value=object()), patch.object(viewers.lifecycle, "healthy", return_value=True), \
             patch.object(display, "geometry", return_value=(1280, 800)), patch.object(display, "resize") as resize, \
             patch.object(display, "refresh_stream", refresh), patch.object(viewers, "publish") as publish:
            with self.assertRaisesRegex(display.DisplayError, "apply failed"):
                display.apply(1024, 768)
            self.assertEqual([call.args for call in resize.call_args_list], [(1024, 768), (1280, 800)])
            self.assertEqual([call.args[1:] for call in refresh.call_args_list], [(1024, 768), (1280, 800)])
            publish.assert_called_once()


class FramebufferBoundaryTests(unittest.TestCase):
    def observer(self, payload):
        import canonical_vnc
        from unittest.mock import Mock
        observer = canonical_vnc.VncGeometry.__new__(canonical_vnc.VncGeometry)
        observer.pixel_bytes = 4
        observer.discarded_pixel_bytes = 0
        observer.size = (1280, 800)
        observer.socket = Mock()
        chunks = bytearray(payload)
        def read(count):
            result = bytes(chunks[:count]); del chunks[:count]
            self.assertEqual(len(result), count)
            return result
        observer.read = read
        return observer

    def test_existing_framebuffer_geometry_is_metadata_only(self):
        import struct
        observer = self.observer(b"\x00\x00\x00\x01" + struct.pack(">HHHHi", 0, 0, 1024, 768, -223))
        observer.update()
        self.assertEqual(observer.size, (1024, 768))
        self.assertEqual(observer.discarded_pixel_bytes, 0)

    def test_oversized_raw_tile_is_refused_before_reading_pixels(self):
        import struct
        import canonical_vnc
        observer = self.observer(b"\x00\x00\x00\x01" + struct.pack(">HHHHi", 0, 0, 33, 32, 0))
        with self.assertRaises(canonical_vnc.VncGeometryError):
            observer.update()
        self.assertEqual(observer.discarded_pixel_bytes, 0)

    def test_update_request_has_no_input_messages(self):
        import struct
        observer = self.observer(b"")
        observer.request_update()
        observer.socket.sendall.assert_called_once_with(struct.pack(">BBHHHH", 3, 1, 0, 0, 1, 1))


if __name__ == "__main__":
    unittest.main()

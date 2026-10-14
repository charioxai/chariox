"""MP-08/MP-10/MP-11: await the focused client's native keymap response.

Observe only keyboard-map requests/replies on the private owned X11 display. Never
record key events, clipboard, window properties, pixels or application text.
"""
import importlib.util
from pathlib import Path
import struct
import sys
import threading
import Xlib.threaded
from Xlib import display
from Xlib.ext import record, res

spec = importlib.util.spec_from_file_location('native_x11', Path(__file__).with_name('native-x11.py'))
x11 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(x11)


def keymap_contains(data, order, request, code, keysym):
    """Check the recycled row, not an unrelated occurrence in a whole map."""
    try:
        if len(data) < 32 or len(data) != 32 + struct.unpack_from(order + 'I', data, 4)[0] * 4:
            return False
        core, first, count = request
        if core:
            width = data[1]
            if not width or not first <= code < first + count:
                return False
            offset = 32 + (code - first) * width * 4
            symbols = struct.unpack_from(order + 'I' * width, data, offset)
        else:
            if len(data) < 40:
                return False
            present = struct.unpack_from(order + 'H', data, 12)[0]
            if not present & 2:
                return False
            offset = 40
            if present & 1:
                for _ in range(data[15]):
                    entries, preserve = data[offset + 5:offset + 7]
                    offset += 8 + entries * (12 if preserve else 8)
            first, count = data[17], data[20]
            if not first <= code < first + count:
                return False
            for current in range(first, code + 1):
                size = struct.unpack_from(order + 'H', data, offset + 6)[0]
                offset += 8
                symbols = struct.unpack_from(order + 'I' * size, data, offset)
                offset += size * 4
        return keysym in symbols and all(symbol in (0, keysym) for symbol in symbols)
    except (IndexError, struct.error):
        return False


class KeymapFence:
    def __init__(self, connection):
        self.control = x11.open_display(display)
        self.data = x11.open_display(display)
        self.lock = threading.Lock()
        self.ready = threading.Event()
        self.received = threading.Event()
        self.wanted = None
        self.failed = False
        self.requests = {}
        self.context = None
        self.thread = None
        try:
            focus = connection.get_input_focus().focus
            if not hasattr(focus, 'id') or focus.id <= 1:
                raise ValueError('native text focus unavailable')
            self.pid = self.client_pid(focus.id)
            if self.pid is None or self.pid <= 1:
                raise ValueError('native text client unavailable')
            ids = self.control.res_query_client_ids([{'client': 0, 'mask': res.LocalClientPIDMask}]).ids
            self.matched = [item.spec.client for item in ids if item.value and item.value[0] == self.pid]
            major = self.control.query_extension('XKEYBOARD').major_opcode
            ranges = {'core_requests': (101, 101), 'core_replies': (101, 101),
                      'ext_requests': (major, major, 8, 8), 'ext_replies': (major, major, 8, 8),
                      'delivered_events': (0, 0), 'device_events': (0, 0),
                      'errors': (0, 0), 'client_started': False, 'client_died': False}
            self.context = self.data.record_create_context(0, self.matched, [ranges])
            self.data.sync()
            self.thread = threading.Thread(target=self.run, daemon=True)
            self.thread.start()
            if not self.ready.wait(1) or self.failed:
                raise ValueError('native keymap observation unavailable')
        except Exception:
            self.close()
            raise

    def client_pid(self, client):
        ids = self.control.res_query_client_ids([{'client': client, 'mask': res.LocalClientPIDMask}]).ids
        return next((item.value[0] for item in ids if item.value), None)

    def reply(self, value):
        if value.category == record.StartOfData:
            self.ready.set()
            return
        with self.lock:
            if value.id_base not in self.matched or len(value.data) > 65536:
                return
            order = '>' if value.client_swapped == (sys.byteorder == 'little') else '<'
            data = value.data
            if value.category == record.FromClient and len(data) >= 8:
                key = (value.id_base, value.recorded_sequence_number & 0xffff)
                self.requests[key] = (data[0] == 101, data[4], data[5])
                if len(self.requests) > 512:
                    self.failed = True
                    self.received.set()
                return
            if value.category != record.FromServer or len(data) < 32:
                return
            key = (value.id_base, struct.unpack_from(order + 'H', data, 2)[0])
            request = self.requests.pop(key, None)
            if self.wanted is not None and request is not None and keymap_contains(data, order, request, self.code, self.wanted):
                self.received.set()

    def run(self):
        try:
            self.data.record_enable_context(self.context, self.reply)
        except Exception:
            self.failed = True
            self.ready.set()
            self.received.set()

    def arm(self, code, keysym):
        with self.lock:
            self.code = code
            self.wanted = keysym
            self.received.clear()

    def wait(self):
        if not self.received.wait(2) or self.failed:
            raise ValueError('native client keymap did not settle')

    def close(self):
        with self.lock:
            self.wanted = None
        try:
            if self.context is not None:
                self.control.record_disable_context(self.context)
                self.control.sync()
                if self.thread is not None:
                    self.thread.join(1)
                self.control.record_free_context(self.context)
                self.control.sync()
                self.context = None
        finally:
            self.data.close()
            self.control.close()

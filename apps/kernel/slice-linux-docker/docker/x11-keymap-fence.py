"""MP-08/MP-10/MP-11: await the focused client's native keymap response.

Observe only keyboard-map requests/replies on the private owned X11 display. Never
record key events, clipboard, window properties, pixels or application text.
"""
import importlib.util
from pathlib import Path
import struct
import sys
import threading
import secrets
import Xlib.threaded
from Xlib import X, display
from Xlib.protocol import event
from Xlib.ext import record, res

spec = importlib.util.spec_from_file_location('native_x11', Path(__file__).with_name('native-x11.py'))
x11 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(x11)


def recorded_messages(value, order):
    """Split RECORD buffers; element sequences use the recorder's byte order."""
    if value.element_header != record.FromClientSequence:
        return
    data = value.data
    offset = 0
    while offset < len(data):
        sequence = None
        if value.category == record.FromClient:
            if len(data) - offset < 8:
                return
            sequence = struct.unpack_from('=I', data, offset)[0] & 0xffff
            offset += 4
            size = struct.unpack_from(order + 'H', data, offset + 2)[0] * 4
            if size < 4:
                return
        elif value.category == record.FromServer:
            if len(data) - offset < 32 or data[offset] != 1:
                return
            size = 32 + struct.unpack_from(order + 'I', data, offset + 4)[0] * 4
            sequence = struct.unpack_from(order + 'H', data, offset + 2)[0]
        else:
            return
        if size > len(data) - offset:
            return
        yield sequence, data[offset:offset + size]
        offset += size


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
        self.ping_received = threading.Event()
        self.ping_nonce = None
        self.wanted = None
        self.failed = False
        self.requests = {}
        self.context = None
        self.thread = None
        try:
            focus = connection.get_input_focus().focus
            if not hasattr(focus, 'id') or focus.id <= 1:
                raise ValueError('native text focus unavailable')
            owner = self.control.res_query_client_ids([{'client': focus.id, 'mask': res.LocalClientPIDMask}]).ids
            if len(owner) != 1 or not owner[0].value or owner[0].value[0] <= 1:
                raise ValueError('native text client unavailable')
            self.pid = owner[0].value[0]
            self.focus_client = owner[0].spec.client
            self.protocol_atom = self.control.intern_atom('WM_PROTOCOLS')
            self.ping_atom = self.control.intern_atom('_NET_WM_PING')
            self.window = self.control.create_resource_object('window', focus.id)
            for _ in range(32):
                protocols = self.window.get_full_property(self.protocol_atom, X.AnyPropertyType)
                if protocols is not None and self.ping_atom in protocols.value:
                    break
                parent = self.window.query_tree().parent
                if parent.id <= 1 or parent.id == self.window.id:
                    raise ValueError('native text delivery acknowledgement unavailable')
                self.window = parent
            else:
                raise ValueError('native text delivery acknowledgement unavailable')
            ids = self.control.res_query_client_ids([{'client': 0, 'mask': res.LocalClientPIDMask}]).ids
            self.matched = [item.spec.client for item in ids if item.value and item.value[0] == self.pid]
            self.major = self.control.query_extension('XKEYBOARD').major_opcode
            ranges = {'core_requests': (101, 101), 'core_replies': (101, 101),
                      'ext_requests': (self.major, self.major, 8, 8), 'ext_replies': (self.major, self.major, 8, 8),
                      'delivered_events': (0, 0), 'device_events': (0, 0),
                      'errors': (0, 0), 'client_started': False, 'client_died': False}
            # Observe only map replies and the application's protocol replies;
            # no delivered key events or application content are recorded.
            ping_range = dict(ranges, core_requests=(25, 25), core_replies=(0, 0),
                              ext_requests=(0, 0, 0, 0), ext_replies=(0, 0, 0, 0))
            # Selected requests may have unobserved requests between them; the
            # buffer's first sequence cannot correlate every map response.
            self.context = self.data.record_create_context(record.FromClientSequence, self.matched, [ranges, ping_range])
            self.data.sync()
            self.thread = threading.Thread(target=self.run, daemon=True)
            self.thread.start()
            if not self.ready.wait(1) or self.failed:
                raise ValueError('native keymap observation unavailable')
        except Exception:
            self.close()
            raise

    def reply(self, value):
        if value.category == record.StartOfData:
            self.ready.set()
            return
        with self.lock:
            if value.id_base not in self.matched or len(value.data) > 65536:
                return
            order = '>' if value.client_swapped == (sys.byteorder == 'little') else '<'
            for sequence, data in recorded_messages(value, order):
                key = (value.id_base, sequence)
                if value.category == record.FromClient:
                    if data[0] == 25:
                        if (len(data) == 44 and value.id_base == self.focus_client
                                and data[12] & 0x7f == X.ClientMessage and data[13] == 32
                                and struct.unpack_from(order + 'I', data, 20)[0] == self.protocol_atom
                                and struct.unpack_from(order + 'III', data, 24) ==
                                (self.ping_atom, self.ping_nonce, self.window.id)):
                            self.ping_received.set()
                    elif data[0] == 101 and len(data) == 8:
                        self.requests[key] = (True, data[4], data[5])
                    elif data[0] == self.major and data[1] == 8 and len(data) == 28:
                        self.requests[key] = (False, 0, 0)
                    if len(self.requests) > 512:
                        self.failed = True
                        self.received.set()
                        return
                else:
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

    def delivered(self):
        # WM_PROTOCOLS events follow the physical key events on the focused
        # client's queue. Its echoed nonce attests consumption before recycling
        # a mapping; a server sync or a mapping reply alone cannot do that.
        with self.lock:
            self.ping_nonce = secrets.randbits(32)
            self.ping_received.clear()
            message = event.ClientMessage(window=self.window.id, client_type=self.protocol_atom,
                data=(32, [self.ping_atom, self.ping_nonce, self.window.id, 0, 0]))
        self.window.send_event(message, event_mask=X.NoEventMask)
        self.control.flush()
        if not self.ping_received.wait(2) or self.failed:
            raise ValueError('native text delivery was not acknowledged')

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

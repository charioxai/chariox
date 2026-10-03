"""Loopback RFB geometry observer. Sends no pointer, keyboard or clipboard input."""
import socket
import struct


class VncGeometryError(RuntimeError):
    pass


class VncGeometry:
    def __init__(self, port, remaining):
        if type(port) is not int or not 1 <= port <= 65535:
            raise VncGeometryError("invalid private framebuffer port")
        self.remaining = remaining
        self.socket = socket.create_connection(("127.0.0.1", port), timeout=min(2, remaining()))
        try:
            if self.read(12) != b"RFB 003.008\n":
                raise VncGeometryError("unsupported private framebuffer version")
            self.socket.sendall(b"RFB 003.008\n")
            count = self.read(1)[0]
            if not count or 1 not in self.read(count):
                raise VncGeometryError("private framebuffer authentication unavailable")
            self.socket.sendall(b"\x01")
            if self.read(4) != b"\x00" * 4:
                raise VncGeometryError("private framebuffer authentication refused")
            self.socket.sendall(b"\x01")  # Shared observer, never displace viewers.
            init = self.read(24)
            self.size = struct.unpack(">HH", init[:4])
            self.pixel_bytes = init[4] // 8
            self.discarded_pixel_bytes = 0
            if self.pixel_bytes not in (1, 2, 4):
                raise VncGeometryError("unsupported private framebuffer pixel format")
            length = struct.unpack(">I", init[20:24])[0]
            if length > 4096:
                raise VncGeometryError("private framebuffer name too large")
            self.read(length)
            # Subscribe only to DesktopSize, not desktop image encodings.
            self.socket.sendall(struct.pack(">BBHi", 2, 0, 1, -223))
        except BaseException:
            self.socket.close()
            raise

    def read(self, length):
        result = bytearray()
        while len(result) < length:
            self.socket.settimeout(min(2, self.remaining()))
            part = self.socket.recv(length - len(result))
            if not part:
                raise VncGeometryError("private framebuffer closed")
            result.extend(part)
        return bytes(result)

    def request_update(self):
        self.socket.sendall(struct.pack(">BBHHHH", 3, 1, 0, 0, 1, 1))

    def prime(self):
        self.socket.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, 1, 1))
        self.update()

    def update(self):
        if self.read(1) != b"\x00":
            raise VncGeometryError("unexpected private framebuffer message")
        _, count = struct.unpack(">BH", self.read(3))
        if count > 16:
            raise VncGeometryError("private framebuffer rectangle count exceeded")
        for _ in range(count):
            _, _, width, height, encoding = struct.unpack(">HHHHi", self.read(12))
            if encoding == -223:
                self.size = width, height
            elif encoding == 0 and width <= 32 and height <= 32 and self.discarded_pixel_bytes + width * height * self.pixel_bytes <= 4096:
                length = width * height * self.pixel_bytes
                self.read(length)
                self.discarded_pixel_bytes += length
            else:
                raise VncGeometryError(f"unexpected private framebuffer encoding {encoding} rectangle {width}x{height}")

    def expect_resize(self, width, height):
        # A one-pixel request may expand to a server tile. Its capped raw bytes are
        # discarded, never logged. Only DesktopSize proves a changed geometry.
        while self.size != (width, height):
            self.socket.sendall(struct.pack(">BBHHHH", 3, 1, 0, 0, 1, 1))
            self.update()

    def close(self):
        self.socket.close()

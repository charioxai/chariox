"""MD-DISPLAY-02/04: sparse bounds come from the complete readback, not events."""

import ctypes
import hashlib
import struct

_memcmp = ctypes.CDLL(None).memcmp
_memcmp.restype = ctypes.c_int
_memcmp.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t]


class RasterComparison:
    # Keep both immutable byte strings alive across the synchronous C compares.
    # Avoid copying two complete rasters into Python row/band slices.
    def __init__(self, previous, raw):
        self.previous, self.raw = previous, raw
        self.before = ctypes.cast(ctypes.c_char_p(previous), ctypes.c_void_p).value
        self.after = ctypes.cast(ctypes.c_char_p(raw), ctypes.c_void_p).value

    def equal(self, start, end):
        return _memcmp(self.before+start, self.after+start, end-start) == 0


class RasterFingerprint:
    # SHA256 over geometry and ordered SHA256 band digests binds every byte.
    # Reuse a band digest only after comparing its actual complete readback.
    # XDamage/event rectangles never authorize reuse. Dense motion rehashes all.
    def __init__(self, width, height):
        self.width, self.height = width, height
        self.band = width*4*64
        self.previous = None
        self.digests = []
        self.compared_previous = None
        self.changed_bands = []

    def update(self, raw):
        if len(raw) != self.width*self.height*4:
            raise ValueError('fingerprint raster size')
        compare = RasterComparison(self.previous, raw) if self.previous is not None else None
        view = memoryview(raw)
        digests = []
        changed = []
        for n, start in enumerate(range(0, len(raw), self.band)):
            end = min(len(raw), start+self.band)
            if compare and compare.equal(start, end):
                digests.append(self.digests[n])
            else:
                changed.append((start, end))
                digests.append(hashlib.sha256(view[start:end]).digest())
        self.compared_previous, self.changed_bands = self.previous, changed
        # An unchanged readback must not replace the last authoritative byte
        # object: the next published sparse packet still follows that snapshot.
        if changed:
            self.previous = raw
        self.digests = digests
        return hashlib.sha256(b'chariox-native-raster-v1' +
                              struct.pack('!II', self.width, self.height) + b''.join(digests)).hexdigest()


def raster_payload(previous, raw, width, height, event_area, fingerprint=None):
    # XDamage wakes capture only. A paint can land after draining those events
    # but before XShmGetImage, so event_area cannot bound this raster's changes.
    if previous is None or len(previous) != len(raw):
        return None, raw
    stride = width*4
    compare = RasterComparison(previous, raw)
    first, last = 0, height
    bound = False
    # These bands were compared byte-for-byte during THIS full fingerprint.
    # Reuse the proven unchanged prefix/suffix instead of rescanning many MiB
    # at every binary-search step. Event hints never authorize this shortcut.
    if (fingerprint is not None and fingerprint.compared_previous is previous
            and fingerprint.previous is raw and fingerprint.width == width
            and fingerprint.height == height):
        if not fingerprint.changed_bands:
            return None, raw
        first = fingerprint.changed_bands[0][0] // stride
        last = fingerprint.changed_bands[-1][1] // stride
        bound = True
    # Binary-search equal complete row prefixes/suffixes before sparse block
    # comparison. This bounds ctypes/GIL crossings even at native DPR2.
    lo, hi = first, min(last, first+64) if bound else last
    while lo < hi:
        mid = (lo+hi)//2
        if compare.equal(first*stride, (mid+1)*stride):
            lo = mid+1
        else:
            hi = mid
    top = lo
    if top == last:
        return None, raw
    lo, hi = max(top, last-64) if bound else top, last-1
    while lo < hi:
        mid = (lo+hi+1)//2
        if compare.equal(mid*stride, last*stride):
            hi = mid-1
        else:
            lo = mid
    bottom = lo+1
    # Conservative whole-frame fallback keeps dense motion's comparison cheap.
    if bottom-top > height*.15:
        return None, raw
    left, right = width, 0
    for row in range(top, bottom):
        offset = row*stride
        if compare.equal(offset, offset+stride):
            continue
        # Binary-search complete row prefixes/suffixes at16px alignment.
        # A small top-right update must not require hundreds of C calls per row.
        blocks = (width+15)//16
        if not compare.equal(offset, offset+left*4):
            lo, hi = 0, blocks-1
            while lo < hi:
                mid = (lo+hi)//2
                if compare.equal(offset, offset+min(width, (mid+1)*16)*4):
                    lo = mid+1
                else:
                    hi = mid
            left = min(left, lo*16)
        if not compare.equal(offset+right*4, offset+stride):
            lo, hi = 0, blocks-1
            while lo < hi:
                mid = (lo+hi+1)//2
                if compare.equal(offset+mid*16*4, offset+stride):
                    hi = mid-1
                else:
                    lo = mid
            right = max(right, min(width, (lo+1)*16))
        if (right-left)*(bottom-top) > width*height*.15:
            return None, raw
    bounds = [left, top, right, bottom]
    payload = b''.join(raw[(row*width+left)*4:(row*width+right)*4] for row in range(top, bottom))
    return bounds, payload


# MP-10/MP-11: private native geometry, no arbitrary desktop allocation.
def capture_geometry_allowed(width, height):
    return type(width) is int and type(height) is int and (width, height) in ((1280,800),(2560,1600),(1920,1080))

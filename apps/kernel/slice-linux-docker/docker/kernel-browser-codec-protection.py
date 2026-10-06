"""MP-11: fail closed before encoding and publishing lossy mask pixels.
Only trusted kernel masks enter here. No capture or client authority.
"""
import math
import av


def bounds(regions, width, height):
    if not isinstance(regions, list) or len(regions) > 50000:
        raise ValueError('mask metadata unavailable')
    result = []
    for region in regions:
        values = [region[k] for k in ('x', 'y', 'width', 'height')]
        if any(type(v) not in (int, float) or not math.isfinite(v) for v in values):
            raise ValueError('mask geometry unavailable')
        x, y, w, h = values
        if w < 0 or h < 0:
            raise ValueError('mask geometry unavailable')
        result.append((max(0, math.floor(x)), max(0, math.floor(y)),
                       min(width, math.ceil(x + w)), min(height, math.ceil(y + h))))
    return result


def black_input(pixels, width, height, regions):
    if len(pixels) != width * height * 4:
        raise ValueError('mask raster unavailable')
    for left, top, right, bottom in bounds(regions, width, height):
        for y in range(top, bottom):
            for channel in range(3):
                if any(pixels[(y * width + left) * 4 + channel:(y * width + right) * 4:4]):
                    raise ValueError('unmasked codec input')


class DecodedMaskGuard:
    def __init__(self, codec):
        self.decoder = av.CodecContext.create('h264' if codec.startswith('avc1') else 'vp8' if codec == 'vp8' else 'vp9', 'r')
        self.decoder.thread_count = 1

    def safe(self, packet, regions, width, height, *, y=0, source_width=None, source_height=None):
        frames = self.decoder.decode(av.Packet(packet))
        if len(frames) != 1 or frames[0].width != width or frames[0].height != height:
            raise ValueError('mask decode unavailable')
        frame = frames[0].reformat(format='rgb24')
        pixels = bytes(frame.planes[0]); stride = frame.planes[0].line_size
        mapped = []
        for r in regions:
            sx = width / (source_width or width); sy = height / (source_height or height)
            mapped.append(dict(x=r['x'] * sx, y=(r['y'] - y) * sy,
                               width=r['width'] * sx, height=r['height'] * sy))
        for left, top, right, bottom in bounds(mapped, width, height):
            for row in range(top, bottom):
                # Keep headroom below the unchanged client RGB64 gate for
                # decoder/color-conversion variation. Exact inputs are black0.
                if max(pixels[row * stride + left * 3:row * stride + right * 3], default=0) > 32:
                    return False
        return True

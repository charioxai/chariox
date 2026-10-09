"""MD-DISPLAY-02: opt-in portable software encoder; input is already protected PNG.
Requires operator-installed Python3/PyAV. No profiles, CDP, credentials or network.
Each independent keyframe can be dropped/reordered without decoder dependencies.
"""
import base64
import io
import json
import sys
from fractions import Fraction
import av

for line in sys.stdin:
    try:
        request = json.loads(line)
        png = base64.b64decode(request['png'], validate=True)
        if len(png) > 4 * 1024 * 1024:
            raise ValueError('frame bound')
        with av.open(io.BytesIO(png)) as source:
            frame = next(source.decode(video=0))
        if frame.width > 2560 or frame.height > 1600:
            raise ValueError('geometry bound')
        codec = av.CodecContext.create('libvpx-vp9', 'w')
        codec.width, codec.height = frame.width, frame.height
        codec.pix_fmt = 'yuv420p'
        codec.time_base = Fraction(1, 5)
        codec.framerate = Fraction(5, 1)
        codec.bit_rate = request['bitrate']
        codec.thread_count = 2
        codec.options = {'deadline': 'realtime', 'cpu-used': '6', 'lag-in-frames': '0',
                         'crf': '18', 'g': '1'}
        frame = frame.reformat(format='yuv420p')
        frame.pts = 0
        packets = list(codec.encode(frame)) + list(codec.encode(None))
        if len(packets) != 1 or not packets[0].is_keyframe:
            raise ValueError('independent keyframe required')
        print(json.dumps({'data_base64': base64.b64encode(bytes(packets[0])).decode()}), flush=True)
    except Exception:
        # Never emit source payloads or arbitrary library exception text.
        print(json.dumps({'error': 'MD-DISPLAY: protected frame encode failed'}), flush=True)

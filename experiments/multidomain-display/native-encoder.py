#!/usr/bin/env python3
"""MD-DISPLAY-02: disposable host codec probe; framed via JSON-line local IPC."""
import base64
import io
import json
import sys
import time
from fractions import Fraction

import av
from PIL import Image

codec = None
for line in sys.stdin:
    request = json.loads(line)
    try:
        if request.get('reset'):
            codec = None
            result = {'reset': True}
        else:
            start = time.perf_counter()
            image = Image.open(io.BytesIO(base64.b64decode(request['png']))).convert('RGB')
            first = codec is None
            if first:
                codec = av.CodecContext.create('libx264rgb', 'w')
                codec.width, codec.height = image.size
                codec.pix_fmt = 'rgb24'
                codec.time_base = Fraction(1, 1000000)
                codec.framerate = Fraction(30)
                codec.thread_count = 5
                codec.bit_rate = request['bitrate']
                codec.colorspace = 0
                codec.color_range = 2
                codec.color_primaries = 1
                codec.color_trc = 13
                codec.options = {
                    'preset': 'ultrafast', 'tune': 'zerolatency',
                    'crf': str(request['crf']),
                    'x264-params': 'repeat-headers=1:keyint=60:scenecut=0:colormatrix=GBR:fullrange=on:'
                    f"vbv-maxrate={request['bitrate']//1000}:vbv-bufsize={request['bitrate']//500}",
                }
                codec.open()
            frame = av.VideoFrame.from_image(image)
            frame.pts = request['timestamp']
            frame.colorspace = 0
            frame.color_range = 2
            packets = codec.encode(frame)
            result = {'packets': [{'data': base64.b64encode(bytes(p)).decode(),
                                   'key': p.is_keyframe, 'timestamp': p.pts}
                                  for p in packets], 'config': first,
                      'elapsed_ms': (time.perf_counter()-start)*1000,
                      'av': av.__version__, 'library_versions': av.library_versions}
        print(json.dumps({'id': request['id'], **result}), flush=True)
    except Exception as error:
        print(json.dumps({'id': request['id'], 'error': str(error)}), flush=True)

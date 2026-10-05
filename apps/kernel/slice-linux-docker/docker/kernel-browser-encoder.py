"""MD-DISPLAY-04: bounded persistent VP9 stream of protected PNG frames only."""
import base64
import io
import json
import sys
from fractions import Fraction
import av

codec = None
configuration = None
sequence = 0
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
        config = (frame.width, frame.height, request['bitrate'])
        if codec is None or config != configuration or request.get('reset'):
            codec = av.CodecContext.create('libvpx-vp9', 'w')
            codec.width, codec.height = frame.width, frame.height
            codec.pix_fmt = 'yuv420p'
            codec.time_base = Fraction(1, 30)
            codec.framerate = Fraction(30, 1)
            # Reserve nested base64, authenticated envelopes and credit replies.
            codec.bit_rate = int(request['bitrate'] * .45)
            codec.thread_count = 4
            codec.options = {'deadline': 'realtime', 'cpu-used': '8', 'lag-in-frames': '0',
                             'g': '60', 'error-resilient': '1', 'undershoot-pct': '95',
                             'overshoot-pct': '5', 'bufsize': str(int(request['bitrate'] * .1))}
            configuration, sequence = config, 0
        frame = frame.reformat(format='yuv420p')
        # PNG decoders mark every input as I; clear that hint for inter prediction.
        frame.pict_type = av.video.frame.PictureType.NONE
        frame.pts = sequence
        packets = list(codec.encode(frame))
        if len(packets) != 1:
            raise ValueError('one realtime packet required')
        sequence += 1
        print(json.dumps({'data_base64': base64.b64encode(bytes(packets[0])).decode(),
                          'key': packets[0].is_keyframe}), flush=True)
    except Exception:
        codec, configuration = None, None
        print(json.dumps({'error': 'MD-DISPLAY: protected frame encode failed'}), flush=True)

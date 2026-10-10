"""MD-DISPLAY-02: reproducible native-DPR encoder/pipe comparison, not relay proof.
Uses an independently captured public fixture PNG, with deterministic full-view
scroll damage. Identical source frames/bitrate are supplied to every encoder.
"""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import resource
import subprocess
import time
import av

parser = argparse.ArgumentParser()
parser.add_argument('encoder', type=Path)
parser.add_argument('fixture', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--frames', type=int, default=90)
parser.add_argument('--bitrate', type=int, default=16000000)
args = parser.parse_args()
source = next(av.open(str(args.fixture)).decode(video=0)).reformat(format='bgr0')
w, h = source.width, source.height
raw = bytes(source.planes[0])
assert len(raw) == w*h*4 and w <= 2560 and h <= 1600
result = {'item':'MD-DISPLAY-02/04', 'encoder_sha256':hashlib.sha256(args.encoder.read_bytes()).hexdigest(),
          'fixture_sha256':hashlib.sha256(args.fixture.read_bytes()).hexdigest(),
          'geometry':[w,h], 'frames':args.frames, 'bitrate':args.bitrate,
          'limits':'Deterministic wrap scrolling of real fixture pixels; includes Python raw pipe/encode/base64 roundtrip, excludes capture/kernel/relay/client. No GPU claim.', 'cases':[]}
for codec in ['vp09.00.50.08', 'avc1.420033']:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.perf_counter()
    child = subprocess.Popen([os.sys.executable, '-u', str(args.encoder)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    spans, payload, keys = [], 0, 0
    try:
        for n in range(args.frames):
            # Scroll includes dense text at native DPR; no reduced motion raster.
            offset = (n*16 % h)*w*4
            pixels = raw[offset:] + raw[:offset]
            header = {'raw':{'width':w,'height':h,'format':'bgr0','length':len(pixels)}, 'codec':codec,'bitrate':args.bitrate,'reset':n==0}
            at = time.perf_counter()
            child.stdin.write(json.dumps(header).encode()+b'\n');child.stdin.write(pixels);child.stdin.flush()
            reply = json.loads(child.stdout.readline())
            if 'error' in reply:raise RuntimeError(reply['error'])
            spans.append((time.perf_counter()-at)*1000)
            payload += len(base64.b64decode(reply['data_base64']));keys+=reply['key']
    finally:
        child.stdin.close()
        child.wait(timeout=10)
        child.stdout.close()
    elapsed=time.perf_counter()-started
    after=resource.getrusage(resource.RUSAGE_CHILDREN)
    spans.sort()
    result['cases'].append({'codec':codec,'p50_ms':spans[(len(spans)-1)//2],'p95_ms':spans[int((len(spans)-1)*.95)],
        'saturated_fps':len(spans)/elapsed,'encoded_bytes':payload,'keyframes':keys,
        'child_cpu_seconds':after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,
        'child_maxrss_kib':after.ru_maxrss,'duration_s':elapsed})
args.output.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))

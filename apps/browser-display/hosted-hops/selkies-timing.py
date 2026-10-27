# MP-08/MP-10/MP-11: apply numeric-only observer to an OWN pinned Selkies installation.
# Never reads/records input content, URL, headers, credentials or frame payload.
import pathlib,sys,hashlib
source=pathlib.Path(sys.argv[1]);out=pathlib.Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
assert str(out).startswith('/root/.chariox/dev/vidlat/r6/')
p=source/'selkies.py';s=p.read_text();original=hashlib.sha256(s.encode()).hexdigest()
needle="                    item = {'data': memoryview(frame), 'owner': frame,"
assert s.count(needle)==1,'MP-10 pinned callback seam changed'
s=s.replace(needle,"                    _vidlat_note('encoded_callback', frame_id=int(frame.frame_id & 0xFFFF), bytes=len(frame))\n"+needle)
needle='                    def do_fanout():\n';assert s.count(needle)==1
s=s.replace(needle,needle+"                        _vidlat_note('fanout', frame_id=int(frame.frame_id & 0xFFFF), bytes=len(frame))\n")
needle='    def offer(self, item';at=s.find(needle);assert at>=0
# Add logger before runtime classes, keeping any __future__ import at its legal location.
at=s.index('class _VideoRelay:')
s=s[:at]+'''def _vidlat_note(stage, **values):
    import json, time, os
    file = os.environ.get('VIDLAT_SELKIES_TIMING')
    if not file: return
    row = {'stage': stage, 'at_ms': time.time_ns()/1e6}
    row.update({k:v for k,v in values.items() if isinstance(v,(int,float))})
    with open(file, 'a') as f: f.write(json.dumps(row)+'\\n')

'''+s[at:]
(out/'selkies.py').write_text(s)
(out/'source-receipt.json').write_text(__import__('json').dumps({'items':['MP-08','MP-10','MP-11'],'original_sha256':original,'instrumented_sha256':hashlib.sha256(s.encode()).hexdigest(),'qualification':'numeric callback/fanout boundaries only; native capture/encode require pixelflux tracer, not inferred'}))
print('MP-08/MP-10/MP-11 numeric-only Selkies callback instrumentation packaged')

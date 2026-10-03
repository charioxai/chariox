# MP-08/MP-10: fail-first close-code-only wire observer contract.
import importlib.util,json,struct
from pathlib import Path
p=Path(__file__).with_name('room-repetition-private-relay-probe.py')
s=importlib.util.spec_from_file_location('probe',p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
def frame(value,masked=False):
 data=json.dumps(value,separators=(',',':')).encode();head=bytes([129,(128 if masked else 0)|126])+struct.pack('!H',len(data));key=b'1234';return head+(key+bytes(b^key[i%4] for i,b in enumerate(data)) if masked else data)
close={'kind':'daemon_display_tunnel_client_close','stream_id':'display-1','error':{'code':'display_stream_backpressure','message':'not-retained'}}
for masked in [False,True]:assert m.close_codes(frame(close,masked))==[{'kind':close['kind'],'code':'display_stream_backpressure'}]
assert m.close_codes(frame({'kind':'daemon_register','auth_token':'not-retained','data':'display_stream_backpressure'}))==[]
print('MP-08/MP-10 close observer contract GREEN')

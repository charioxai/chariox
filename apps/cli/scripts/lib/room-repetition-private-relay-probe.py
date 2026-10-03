# MP-08/MP-10: only display-close codes from this owned slice's loopback relay.
import json,os,socket,struct,time
KINDS={'daemon_display_tunnel_close','daemon_display_tunnel_client_close'}
def close_codes(data):
 result=[];offset=0
 while offset+2<=len(data):
  if data[offset] not in (129,130):break
  flags=data[offset+1];length=flags&127;offset+=2
  if length==126:
   if offset+2>len(data):break
   length=struct.unpack('!H',data[offset:offset+2])[0];offset+=2
  elif length==127:
   if offset+8>len(data):break
   length=struct.unpack('!Q',data[offset:offset+8])[0];offset+=8
  key=None
  if flags&128:
   if offset+4>len(data):break
   key=data[offset:offset+4];offset+=4
  if offset+length>len(data):break
  body=data[offset:offset+length];offset+=length
  prefix=bytes(b^key[i%4] for i,b in enumerate(body[:100])) if key else body[:100]
  if not any(('"kind":"'+kind+'"').encode() in prefix for kind in KINDS):continue
  if key:body=bytes(b^key[i%4] for i,b in enumerate(body))
  try:value=json.loads(body)
  except (ValueError,UnicodeError):continue
  code=value.get('error',{}).get('code') if value.get('error') else None
  if value.get('kind') in KINDS:result.append({'kind':value['kind'],'code':code})
 return result

def main():
 path='/tmp/loops-private-relay-codes.jsonl';descriptor=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
 with os.fdopen(descriptor,'w',buffering=1) as out:
  try:
   capture=socket.socket(socket.AF_PACKET,socket.SOCK_RAW,socket.htons(3));capture.bind(('lo',0));capture.settimeout(1)
  except OSError as e:out.write(json.dumps({'mpItems':['MP-08','MP-10'],'supported':False,'errorType':type(e).__name__})+'\n');return
  port=int(os.environ['LOOPS_PRIVATE_RELAY_PORT']);out.write(json.dumps({'mpItems':['MP-08','MP-10'],'supported':True,'port':port,'scope':'own loopback; complete close envelopes only; no packet/auth/video payload retained'})+'\n');end=time.monotonic()+180;seen=set()
  while time.monotonic()<end:
   try:data=capture.recv(65535)
   except socket.timeout:continue
   if len(data)<54 or data[12:14]!=b'\x08\x00' or data[23]!=6 or data[26:30]!=b'\x7f\x00\x00\x01' or data[30:34]!=b'\x7f\x00\x00\x01':continue
   tcp=14+(data[14]&15)*4;src,dst=struct.unpack('!HH',data[tcp:tcp+4])
   if port not in (src,dst):continue
   sequence=struct.unpack('!I',data[tcp+4:tcp+8])[0];payload=tcp+(data[tcp+12]>>4)*4
   for value in close_codes(data[payload:]):
    key=(src,dst,sequence,value['kind'],value['code'])
    if key in seen:continue
    seen.add(key);out.write(json.dumps({'mpItems':['MP-08','MP-10'],'at':time.time(),'sourcePort':src,'destinationPort':dst,**value})+'\n')
  capture.close()
if __name__=='__main__':main()

# MP-08/MP-10/MP-11 instrumentation: numeric boundaries, no request/payload logging.
import os,time,json,functools,inspect
path=os.environ.get('VIDLAT_SELKIES_TIMING')
def note(stage,start,**fields):
 if not path:return
 end=time.time_ns()/1e6
 row={'stage':stage,'started_ms':start,'ended_ms':end,'duration_ms':end-start}
 row.update({k:v for k,v in fields.items() if isinstance(v,(int,float))})
 with open(path,'a')as f:f.write(json.dumps(row)+'\n')
def wrap_async(cls,name,stage):
 fn=getattr(cls,name)
 @functools.wraps(fn)
 async def call(self,*args,**kwargs):
  start=time.time_ns()/1e6
  try:return await fn(self,*args,**kwargs)
  finally:
   fields={}
   if stage=='socket_send' and args and isinstance(args[0],(bytes,bytearray,memoryview)) and len(args[0])>=10:
    b=args[0];fields={'frame_id':int(b[2])*256+int(b[3]),'video':1 if b[0] in (3,4) else 0,'bytes':len(b)}
   if stage in ('input_dispatch','input_arrival') and args and isinstance(args[0],str):
    prefix=args[0].split(',',1)[0];fields['kind']={'m':1,'kd':2,'ku':3}.get(prefix,0)
    if prefix=='m':
     try:fields['pressed']=1 if int(args[0].split(',')[3])&1 else 0
     except (ValueError,IndexError):pass
   note(stage,start,**fields)
 setattr(cls,name,call)
if path:
 try:
  from aiohttp.web_ws import WebSocketResponse
  wrap_async(WebSocketResponse,'send_bytes','socket_send')
  from selkies.input_handler import WebRTCInput
  for name,stage in [('on_message','input_arrival'),('_dispatch_message','input_dispatch'),('send_x11_mouse','native_mouse'),('send_x11_keypress','native_key')]:wrap_async(WebRTCInput,name,stage)
  from selkies.stream_server import TransferPacer
  wrap_async(TransferPacer,'pace','pacing')
  note('observer_ready',time.time_ns()/1e6)
 except (ImportError,AttributeError):note('observer_install_failed',time.time_ns()/1e6)

# MP-08/MP-10/MP-11: video queue and ACK gate seams (file TransferPacer excluded).
if path:
 try:
  from selkies.selkies import _VideoRelay, DataStreamingServer
  original_offer=_VideoRelay.offer
  def traced_offer(self,item):
   start=time.time_ns()/1e6;before=len(self.backlog);result=original_offer(self,item)
   note('video_offer',start,frame_id=int(item['frame_id']),backlog_before=before,backlog_after=len(self.backlog),backlog_bytes=self.backlog_bytes,accepted=int(bool(self.backlog) and self.backlog[-1] is item),sync_requested=int(result))
   return result
  _VideoRelay.offer=traced_offer
  original_gate=DataStreamingServer._set_backpressure_enabled
  def traced_gate(self,display_id,state,enabled):
   start=time.time_ns()/1e6;result=original_gate(self,display_id,state,enabled)
   note('video_credit_gate',start,enabled=int(enabled),sent=int(state.get('last_sent_frame_id',0)),acked=int(state.get('acknowledged_frame_id',-1)))
   return result
  DataStreamingServer._set_backpressure_enabled=traced_gate
 except (ImportError,AttributeError):note('video_observer_install_failed',time.time_ns()/1e6)

# MP-08 / MP-10: observe only the drill-owned official provider.
# Provider payloads exist in memory only and are discarded after classification.
import ast,json,re,signal,subprocess,sys
from pathlib import Path
pid=int(sys.argv[1]);assert pid>1
out=Path(sys.argv[2]);assert not out.is_relative_to(Path.cwd())
stdout=str(Path(f'/proc/{pid}/fd/1').readlink())
tracer=subprocess.Popen(['strace','-f','-ttt','-s','65536','-e','trace=write','-p',str(pid)],stderr=subprocess.PIPE,stdout=subprocess.DEVNULL,text=True,errors='replace')
assert tracer.pid>1
stopping=False

def stop(signum,frame):
 global stopping
 stopping=True
 if tracer.poll() is None:tracer.send_signal(signal.SIGINT)
signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
known={'system','user','assistant','result','stream_event','control_request','control_response','keep_alive','rate_limit_event'}
pending='';count=0
try:
 with out.open('w')as stream:
  stream.write(json.dumps({'items':['MP-08','MP-10'],'observer':'owned-provider-protocol-metadata','pid':pid,'tracerPid':tracer.pid})+'\n');stream.flush()
  for line in tracer.stderr:
   if stopping:continue
   # A root thread writing to the root stdout pipe; never hook child pipes.
   m=re.search(r'(?:\[pid\s+(\d+)\]\s+|^(\d+)\s+)?(\d+\.\d+) write\((\d+), ("(?:[^"\\]|\\.)*"), (\d+)\)\s+=\s+(\d+)',line)
   if not m:continue
   tid=int(m[1]or m[2]or pid);fd=int(m[4])
   try:
    status=Path(f'/proc/{tid}/status').read_text()
    if int(re.search(r'^Tgid:\s+(\d+)',status,re.M)[1])!=pid:continue
    if str(Path(f'/proc/{pid}/fd/{fd}').readlink())!=stdout:continue
   except (OSError,AttributeError):continue
   try:payload=ast.literal_eval(m[5])
   except (ValueError,SyntaxError):continue
   if len(pending)+len(payload)>65536:pending='';continue
   pending+=payload
   while '\n' in pending:
    record,pending=pending.split('\n',1)
    try:value=json.loads(record)
    except (ValueError,TypeError):continue
    kind=value.get('type')
    if kind not in known:kind='other'
    event=value.get('event',{});event=event if isinstance(event,dict)else{}
    content=value.get('message',{});content=content if isinstance(content,dict)else{}
    text=''.join(c.get('text','')for c in content.get('content',[])if isinstance(c,dict)and isinstance(c.get('text'),str))
    def tag(value):return value if value in {'hook_started','hook_response','hook_progress','init','status','success','error_during_execution','error_max_turns','error_max_budget_usd','error_max_structured_output_retries','message_start','content_block_start','content_block_delta','content_block_stop','message_delta','message_stop','error'}else None
    row={'items':['MP-08','MP-10'],'atMs':float(m[3])*1000,'type':kind,'subtype':tag(value.get('subtype')),'eventType':tag(event.get('type')),'hookEvent':value.get('hook_event')if value.get('hook_event')in ['SessionStart','UserPromptSubmit','PreToolUse','PostToolUse','Stop']else None,'byteCount':len(record.encode()),'isError':bool(value.get('is_error')),'submittedPromptSeen':'the words tuifix marker seven alpha' in text,'markerSeen':'TUIFIX MARKER' in text or 'TUIFIX MARKER' in str(event.get('delta',{}).get('text',''))}
    if count<200:stream.write(json.dumps(row)+'\n');stream.flush();count+=1
finally:
 if tracer.poll() is None:tracer.send_signal(signal.SIGINT)
 try:tracer.wait(timeout=3)
 except subprocess.TimeoutExpired:tracer.kill();tracer.wait(timeout=3)

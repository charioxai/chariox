# MP-08/MP-10: isolate two existing private adapters; never retain tokens or pixels.
import json,subprocess,threading,time,re
rows=[];children=[];threads=[]
def drain(child,row):
 for line in child.stdout:
  try:value=json.loads(line)
  except (ValueError,UnicodeError):row['malformedRecords']+=1;continue
  kind=value.get('kind');row['records'][kind]=row['records'].get(kind,0)+1
  if kind=='binary':row['videoBase64Bytes']+=len(value.get('data_base64',''));row['firstFrameAt']=row['firstFrameAt'] or time.monotonic()
  if kind=='closed':row['closedReason']=value.get('reason') if value.get('reason') in ['closed','lease_expired'] else 'other'
def launch(index):
 child=subprocess.Popen(['/opt/chariox-selkies/bin/python','/opt/chariox-slice/slice-selkies-stream.py'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 row={'viewer':index,'pid':child.pid,'records':{},'videoBase64Bytes':0,'firstFrameAt':None,'malformedRecords':0};children.append(child);rows.append(row)
 thread=threading.Thread(target=drain,args=(child,row),daemon=True);thread.start();threads.append(thread)
start=time.monotonic();launch(0)
while time.monotonic()-start<10 and not rows[0]['firstFrameAt'] and children[0].poll() is None:time.sleep(.05)
launch(1);samples=[];deadline=time.monotonic()+10
while time.monotonic()<deadline:
 samples.append({'elapsedSeconds':time.monotonic()-start,'exitCodes':[c.poll() for c in children],'videoRecords':[r['records'].get('binary',0) for r in rows]});time.sleep(.2)
for child,row,thread in zip(children,rows,threads):
 row['exitCodeBeforeOwnedClose']=child.poll();child.stdin.close()
 try:child.wait(timeout=6)
 except subprocess.TimeoutExpired:child.kill();child.wait();row['forcedOwnedKill']=True
 thread.join(timeout=2);diagnostic=child.stderr.read().decode(errors='replace')
 row['stderrBytes']=len(diagnostic);row['streamDisconnected']='private stream disconnected' in diagnostic;row['handshakeFailed']='private viewer handshake failed' in diagnostic
 row['failureTypes']=re.findall(r'private Selkies stream ended or failed \(([A-Za-z0-9_]+)\)',diagnostic);row['finalExitCode']=child.returncode
print(json.dumps({'mpItems':['MP-08','MP-10'],'scope':'direct existing private adapters; no kernel authority or encrypted relay exercised','viewers':rows,'samples':samples}))

# MP-08/MP-10/MP-11: owned PTY driver for the real compiled Chariox TUI.
import os,sys,json,pty,fcntl,termios,struct,subprocess,select,socket,time,signal,re,threading
from pathlib import Path
spec=json.loads(sys.stdin.read());root=Path(spec['root']);out=Path(spec['evidence']);master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',45,144,0,0));auto=root/'automation.sock'
env=spec['env'];env['CHARIOX_LIVE_TERMINAL_TOKEN']=spec['token'];env['CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR']=spec['registry']
child=subprocess.Popen([spec['binary'],'--detached','--relay-url',spec['relayUrl'],'--relay-token-env','CHARIOX_LIVE_TERMINAL_TOKEN','--target-daemon-id',spec['kernelId'],'--automation-socket',str(auto)],env=env,stdin=slave,stdout=slave,stderr=slave,cwd=str(root))
assert isinstance(child.pid,int) and child.pid>1
os.close(slave);done=threading.Event();stream=bytearray()
def drain():
 while not done.is_set():
  if not select.select([master],[],[],.1)[0]:continue
  try:data=os.read(master,65536)
  except OSError:break
  if not data:break
  stream.extend(data)
  if b'\x1b[>0q' in data:os.write(master,b'\x1bP>|XTerm(399)\x1b\\')
  for m in re.findall(rb'\x1b\[\?(\d+)\$p',data):os.write(master,b'\x1b[?'+m+b';2$y')
  if b'\x1b[18t' in data:os.write(master,b'\x1b[8;45;144t')
def rpc(value):
 with socket.socket(socket.AF_UNIX) as conn:
  conn.settimeout(15);conn.connect(str(auto));conn.sendall((json.dumps(value)+'\n').encode());data=b''
  while b'\n' not in data:
   part=conn.recv(65536)
   if not part:raise RuntimeError('automation closed')
   data+=part
  return json.loads(data.split(b'\n')[0])
thread=threading.Thread(target=drain,daemon=True);thread.start();result={'items':['MP-08','MP-10','MP-11'],'pid':child.pid,'status':'FAIL','checkpoints':[]}
try:
 for _ in range(200):
  if auto.exists():break
  if child.poll() is not None:raise RuntimeError('TUI exited')
  time.sleep(.1)
 assert rpc({'action':'connect_detached_kernel'}).get('ok') is True
 deadline=time.monotonic()+spec['durationMs']/1000
 while time.monotonic()<deadline:
  snapshot=rpc({'action':'snapshot'});assert snapshot.get('ok') is True
  data=snapshot['data'];status=data.get('statusLine','');result['checkpoints'].append({'at':time.time(),'statusLine':status,'disconnected':data.get('daemonDisconnected')})
  assert not data.get('daemonDisconnected'),'TUI disconnected'
  time.sleep(1)
 text=stream.decode('utf8','replace').replace(spec['token'],'[redacted]')
 # Keep real terminal output; no auth values, tokens or environment are retained.
 text=re.sub(r'\x1b\][^\x07]*(?:\x07|\x1b\\)','',text);text=re.sub(r'\x1bP.*?\x1b\\','',text,flags=re.S)
 (out/'terminal-pty.txt').write_text(text)
 label='Local connect' if spec['expectLocal'] else 'Relay connect'
 assert label in text,'expected carrier indicator absent'
 result['status']='PASS';result['expectedLocal']=spec['expectLocal']
except Exception:
 result['failure']='real detached TUI did not retain the expected carrier and connection';raise
finally:
 done.set()
 if child.poll() is None:
  assert isinstance(child.pid,int) and child.pid>1
  child.send_signal(signal.SIGTERM)
  try:child.wait(timeout=5)
  except subprocess.TimeoutExpired:
   assert child.pid>1;child.send_signal(signal.SIGKILL);child.wait(timeout=5)
 thread.join(timeout=2);os.close(master);result['cleanup']=child.poll() is not None
 (out/'terminal-driver.json').write_text(json.dumps(result,indent=2))
 print('MP-08/MP-10/MP-11 terminal driver '+result['status'])

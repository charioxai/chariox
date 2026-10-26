// MP-08/MP-10/MP-11: own runtime lifecycle, ordinary kernel/browser authority.
import {spawn} from 'node:child_process';
import {mkdir,writeFile} from 'node:fs/promises';
import {openSync,readFileSync,writeFileSync} from 'node:fs';import net from 'node:net';
import {OwnedDisplay} from './owned-display.mjs';
const {AB_KIT:kit,AB_STATE:root,AB_ORIGIN:origin,AB_API_ORIGIN:apiOrigin,AB_RELAY_URL:relay,AB_SELKIES_ORIGIN:selkiesOrigin}=process.env;
if(!kit||!root||!origin||!selkiesOrigin)throw Error('MP-11: A/B configuration required');
const children=[],identities=[];let display,stopping=false;
function remember(pid){
 if(!Number.isSafeInteger(pid)||pid<=1)throw Error('MP-11: unsafe owned child PID');
 const fields=readFileSync(`/proc/${pid}/stat`,'utf8').split(')').at(-1).trim().split(/\s+/);
 identities.push({pid,start:fields[19],ppid:Number(fields[1])});
 writeFileSync(root+'/child-identities.json',JSON.stringify(identities),{mode:0o600});
}
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const free=()=>new Promise((r,j)=>{const s=net.createServer();s.once('error',j);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>r(p))})});
const start=(bin,args,env,name)=>{
 const log=openSync(`${root}/logs/${name}.private.log`,'a',0o600);
 const child=spawn(bin,args,{cwd:root,env,stdio:['ignore',log,log]});children.push(child);if(child.pid!==undefined)remember(child.pid);
 child.once('error',()=>{stopping=true;void writeFile(root+'/FAILED','MP-10 runtime spawn failed');});return child;
};
for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>{stopping=true});
try{
 for(const d of ['V/home','V/runtime','V/config','V/data','V/cache','V/state','V/logs','S/home','S/tmp'])await mkdir(root+'/'+d,{recursive:true,mode:0o700});
 const ports=[];for(let n=0;n<4;n++)ports.push(await free());
 const streamPort=await free(),cdpPort=await free();
 await writeFile(root+'/V/node-userns.sh',`#!/bin/bash\nset -euo pipefail\ncase "$3" in "${root}/V/runtime/"*) ;; *) exit 2;; esac\nexec /usr/bin/unshare --user --map-user=1000 --map-group=1000 -- /usr/bin/node "$@"\n`,{mode:0o700});
 const common={PATH:process.env.PATH,LANG:'C.UTF-8',TMPDIR:root+'/tmp',LD_LIBRARY_PATH:process.env.LD_LIBRARY_PATH};
 const kernel=start(kit+'/bin/chariox-kernel',[],{...common,HOME:root+'/V/home',CHARIOX_HOME:root+'/V/runtime',CHARIOX_LOG_DIR:root+'/V/logs',XDG_CONFIG_HOME:root+'/V/config',XDG_DATA_HOME:root+'/V/data',XDG_CACHE_HOME:root+'/V/cache',XDG_STATE_HOME:root+'/V/state',CHARIOX_KERNEL_HOST:'127.0.0.1',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_DAEMON_ALIAS:'vidlat-owner-ab',CHARIOX_BROWSER_CONTROLLER_NODE:root+'/V/node-userns.sh',CHARIOX_KERNEL_BROWSER_EXECUTABLE:'/usr/bin/google-chrome',CHARIOX_KERNEL_BROWSER_HEADLESS:'0',CHARIOX_KERNEL_BROWSER_DISPLAY:'1',CHARIOX_BROWSER_DISPLAY_SOFTWARE:'1',CHARIOX_BROWSER_DISPLAY_OPENH264:process.env.CHARIOX_BROWSER_DISPLAY_OPENH264},'kernel');
 display=new OwnedDisplay(root+'/S',{width:2560,height:1600});const xenv=await display.start();remember(display.child.pid);
 start('/usr/bin/unshare',['--user','--map-user=1000','--map-group=1000','--','/usr/bin/openbox','--sm-disable'],{...common,...xenv,HOME:root+'/S/home',TMPDIR:root+'/S/tmp',XDG_CONFIG_HOME:root+'/S/home/.config'},'selkies-wm');await delay(500);
 start('/usr/bin/unshare',['--user','--map-user=1000','--map-group=1000','--','/opt/google/chrome/chrome','--user-data-dir='+root+'/S/home/chrome','--remote-debugging-port='+cdpPort,'--remote-debugging-address=127.0.0.1','--no-first-run','--disable-gpu','--disable-dev-shm-usage','--disable-lcd-text','--force-device-scale-factor=2','--disable-renderer-backgrounding','--disable-background-timer-throttling','--disable-backgrounding-occluded-windows','--window-position=0,0','--window-size=1280,800','https://en.wikipedia.org/wiki/Web_browser'],{...common,...xenv,HOME:root+'/S/home',TMPDIR:root+'/S/tmp'},'selkies-chrome');
 // Restrict the only non-loopback listener BEFORE starting it; own exact rule.
 if(process.env.AB_REHEARSAL!=='1'){
  const firewall=spawn('python3',[kit+'/ab-processes.py','firewall',String(streamPort)],{env:process.env,stdio:'ignore'});
  if(await new Promise(r=>firewall.once('exit',r))!==0)throw Error('MP-11: baseline firewall admission failed');
 }
 start('/usr/bin/unshare',['--user','--map-user=1000','--map-group=1000','--',root+'/python/bin/python','-m','selkies','--addr='+(process.env.AB_REHEARSAL==='1'?'127.0.0.1':'0.0.0.0'),'--port='+streamPort,'--mode=websockets','--enable-https=false','--encoder=h264enc','--use-cpu=true|locked','--framerate=60-60','--rate-control-mode=cbr','--video-bitrate=8000-8000','--manual-resolution=true|locked','--manual-width=2560','--manual-height=1600','--enable-resize=false|locked','--audio-enabled=false|locked','--microphone-enabled=false|locked','--gamepad-enabled=false|locked','--webcam-enabled=false|locked','--enable-clipboard=false','--enable-basic-auth=false','--allowed-origins='+selkiesOrigin],{...common,...xenv,HOME:root+'/S/home',TMPDIR:root+'/S/tmp'},'selkies');
 await writeFile(root+'/runtime.json',JSON.stringify({items:['MP-08','MP-10','MP-11'],ports,streamPort,cdpPort,origin,apiOrigin,relay,selkiesOrigin,kernelPid:kernel.pid}),{mode:0o600});
 for(let n=0;n<240&&!stopping;n++){
  if(children.some(c=>c.exitCode!==null||c.signalCode!==null))throw Error('MP-10: runtime child exited');
  try{const response=await fetch(`http://127.0.0.1:${streamPort}/`);if(response.ok){await writeFile(root+'/READY','MP-08/MP-10/MP-11 runtime ready');break}}catch{}
  await delay(250);
 }
 while(!stopping){if(children.some(c=>c.exitCode!==null||c.signalCode!==null))throw Error('MP-10: runtime child exited');await delay(250)}
}catch(e){await writeFile(root+'/FAILED','MP-08/MP-10/MP-11: '+e.message);throw e}
finally{
 for(const child of children.reverse()){
  if(child.pid===undefined)continue;
  if(!Number.isSafeInteger(child.pid)||child.pid<=1)throw Error('MP-11 unsafe owned PID');
  if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');
  for(let n=0;n<40&&child.exitCode===null&&child.signalCode===null;n++)await delay(100);
  if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL');
 }
 await display?.close();
}

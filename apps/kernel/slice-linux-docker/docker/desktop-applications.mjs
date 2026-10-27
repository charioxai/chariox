// MP-08/MP-11: resolve installed IDs; launch/cleanup stays with LinuxOwnedDesktop.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { processIdentity, settleOwned } from './linux-owned-process.mjs';
export async function desktopApplication(request, environment, signal) {
  if(signal?.aborted)throw Error('MP-11: application request cancelled');
  const child=spawn('/usr/bin/python3',[fileURLToPath(new URL('./desktop-applications.py',import.meta.url))],{env:environment,stdio:['pipe','pipe','ignore']});
  child.on('error',()=>{});child.stdin.on('error',()=>{});
  const closed=new Promise(resolve=>{child.once('close',resolve);child.once('error',()=>resolve(-1));});
  const identity=await processIdentity(child.pid);
  if(!identity){await closed;throw Error('MP-11: application resolver ownership unavailable');}
  let output='',overflow=false;
  const stop=()=>void settleOwned([{child,identity}]);
  child.stdout.on('data',chunk=>{output+=chunk;if(Buffer.byteLength(output)>128*1024){overflow=true;stop();}});
  signal?.addEventListener('abort',stop,{once:true});const timer=setTimeout(stop,5000);
  try {
    if(signal?.aborted)stop();
    child.stdin.end(JSON.stringify(request));
    const code=await closed;
    if(code!==0 || overflow || signal?.aborted)throw Error('MP-08: installed application refused');
    return JSON.parse(output);
  } finally {clearTimeout(timer);signal?.removeEventListener('abort',stop);await settleOwned([{child,identity}]);}
}

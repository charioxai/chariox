// MP-08/MP-10/MP-11: numeric inventory on the kernel's owned private display.
// This does not grant a window/input capability or classify an auxiliary popup.
import {spawn} from 'node:child_process';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
export async function nativeWindowInventory({display,pid,isCurrent}){
 const current=()=>ownsDisplay(display)&&Number.isSafeInteger(pid)&&pid>1&&typeof isCurrent==='function'&&isCurrent()===true;
 if(!current()||!process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER)throw Error('MP-11: native window inventory lease');
 const child=spawn(process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER,['--display-native-windows'],{env:{...process.env,...display.environment},stdio:['pipe','pipe','ignore']});
 let bytes='',timer;
 try{
  const rows=await new Promise((resolve,reject)=>{
   const refuse=()=>reject(Error('MP-11: native window inventory unavailable'));
   timer=setTimeout(refuse,5000);child.once('error',refuse);child.stdin.once('error',refuse);
   child.stdout.on('data',data=>{bytes+=data;if(bytes.length>32768)refuse()});
   child.once('close',code=>{try{if(code!==0||bytes.length>32768)throw Error();resolve(JSON.parse(bytes))}catch{refuse()}});
   child.stdin.end(JSON.stringify({pid}));
  });
  if(!current())throw Error('MP-11: native window inventory binding');
  return checkedInventory(rows);
 }finally{
  clearTimeout(timer);
  if(child.pid!==undefined){if(!Number.isSafeInteger(child.pid)||child.pid<=1)throw Error('MP-11: unsafe native inventory PID');
   if(child.exitCode===null&&child.signalCode===null){const exited=new Promise(r=>child.once('exit',r));child.kill('SIGTERM');await Promise.race([exited,new Promise(r=>setTimeout(r,1000))]);if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await exited;}}
  }
 }
}
export function checkedInventory(rows){
 if(!Array.isArray(rows)||rows.length>32||rows.some(r=>!r||
  Object.keys(r).sort().join(',')!=='focused,frame_insets,height,id,override_redirect,transient_for,visible,width,x,y'||
  !Array.isArray(r.frame_insets)||r.frame_insets.length!==4||!r.frame_insets.every(n=>Number.isSafeInteger(n)&&n>=0&&n<=64)||
  ![r.id,r.width,r.height,r.x,r.y,r.transient_for].every(Number.isSafeInteger)||r.id<=1||r.id>0xffffffff||
  r.width<1||r.width>8192||r.height<1||r.height>8192||r.transient_for<0||r.transient_for>0xffffffff||
  ![r.visible,r.override_redirect,r.focused].every(v=>typeof v==='boolean')))throw Error('MP-11: native window inventory binding');
 return rows.map(r=>Object.freeze({...r,frame_insets:Object.freeze(r.frame_insets)}));
}

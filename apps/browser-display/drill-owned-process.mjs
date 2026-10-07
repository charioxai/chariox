// MD-DISPLAY-02/03: signal only children launched here, bound to Linux start time.
import {spawn} from 'node:child_process';
import {readFile,readdir} from 'node:fs/promises';
const owned=new WeakMap();
const pause=ms=>new Promise(r=>setTimeout(r,ms));
export const validPid=pid=>Number.isSafeInteger(pid)&&pid>1&&pid!==process.pid;
async function identity(pid){
 try{const s=await readFile(`/proc/${pid}/stat`,'utf8'),f=s.slice(s.lastIndexOf(')')+2).split(' ');return{pid,ppid:Number(f[1]),group:Number(f[2]),session:Number(f[3]),start:f[19],state:f[0]}}catch(e){if(e.code==='ENOENT'||e.code==='ESRCH')return null;throw e}
}
export async function launchOwned(command,args,options={}){
 const child=spawn(command,args,options),record={pid:null,detached:options.detached===true,members:new Map(),error:null};owned.set(child,record);
 record.completion=new Promise(resolve=>child.once('close',(code,signal)=>resolve({code,signal})));

 // Keep the error listener for the lifetime of the child, including failed spawn.
 child.on('error',e=>{record.error=e});
 await new Promise((resolve,reject)=>{child.once('spawn',resolve);child.once('error',reject)});
 if(!validPid(child.pid))throw Error('MD-DISPLAY refused unsafe child PID');
 record.pid=child.pid;const p=await identity(child.pid);if(p)record.members.set(p.pid,p.start);
 return child;
}
export const waitChild=child=>owned.get(child).completion;
export function checkChild(child,label='child'){
 const record=owned.get(child);if(record?.error)throw record.error;
 if(child.exitCode!==null||child.signalCode!==null)throw Error(`MD-DISPLAY ${label} exited ${child.exitCode??child.signalCode}`);
}
export async function rememberGroup(child){
 const record=owned.get(child);if(!record||!validPid(record.pid)||!record.detached)return[];
 const members=[];
 for(const name of await readdir('/proc'))if(/^\d+$/.test(name)){const p=await identity(Number(name));if(p?.group===record.pid)members.push(p)}
 const known=new Set(members.filter(p=>record.members.get(p.pid)===p.start).map(p=>p.pid));
 let changed=true;while(changed){changed=false;for(const p of members)if(!known.has(p.pid)&&known.has(p.ppid)){known.add(p.pid);changed=true}}
 if(members.some(p=>!validPid(p.pid)||p.session!==record.pid||!known.has(p.pid)))throw Error('MD-DISPLAY refused unowned or reused process group');
 for(const p of members)record.members.set(p.pid,p.start);
 return members;
}
export async function signalChild(child,signal='SIGTERM'){
 const record=owned.get(child);if(!record||!validPid(record.pid))return false;
 const p=await identity(record.pid);if(!p)return false;
 if(record.members.get(p.pid)!==p.start)throw Error('MD-DISPLAY refused reused child PID');
 process.kill(p.pid,signal);return true;
}
export async function stopGroup(child){
 const record=owned.get(child);if(!record||!validPid(record.pid)||!record.detached)return;
 for(const signal of ['SIGTERM','SIGKILL']){
  // Recheck every member and its ownership before each negative group signal.
  const members=await rememberGroup(child);if(!members.some(p=>p.state!=='Z'))return;
  if(!validPid(record.pid))throw Error('MD-DISPLAY refused unsafe group PID');
  try{process.kill(-record.pid,signal)}catch(e){if(e.code!=='ESRCH')throw e}
  await pause(signal==='SIGTERM'?200:100);
 }
}

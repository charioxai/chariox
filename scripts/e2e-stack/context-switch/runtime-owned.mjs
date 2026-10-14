// MP-08 / MP-10 / MP-11: find detached children by exact disposable runtime paths, never process names.
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {basename,dirname,isAbsolute} from 'node:path';
import {createOwnedSignalGuard,processSnapshot} from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs';
const keys=['CHARIOX_HOME','HOME','CODEX_HOME','CLAUDE_CONFIG_DIR','OPENCODE_CONFIG_DIR','XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_STATE_HOME','XDG_CACHE_HOME'];
export async function createOwnedRuntime(root,{name,head}){
 if(!isAbsolute(root)||!/^[-\w]+$/.test(name)||basename(root)!=='run-'+name)throw Error('Invalid disposable runtime root');
 await mkdir(dirname(root),{recursive:true,mode:0o700});await mkdir(root,{mode:0o700});
 await writeFile(root+'/context-switch-owned.json',JSON.stringify({name,head}),{flag:'wx',mode:0o600});
}
async function matching(root){
 const rows=[];
 for(const row of processSnapshot()){
  let env;try{env=new Map((await readFile('/proc/'+row.pid+'/environ')).toString().split('\0').map(x=>{const at=x.indexOf('=');return [x.slice(0,at),x.slice(at+1)]}));}catch{continue;}
  const key=keys.find(k=>env.get(k)===root||env.get(k)?.startsWith(root+'/'));if(!key)continue;
  if(!Number.isInteger(row.pid)||row.pid<=1)throw Error('Unsafe runtime PID');
  if(processSnapshot().some(p=>p.pid===row.pid&&p.start===row.start&&p.state!=='Z'))rows.push({pid:row.pid,start:row.start,ownershipKey:key});
 }
 return rows;
}
export async function sweepOwnedRuntime(root){
 const marker=JSON.parse(await readFile(root+'/context-switch-owned.json','utf8'));
 if(!isAbsolute(root)||!/^[-\w]+$/.test(marker.name)||basename(root)!=='run-'+marker.name)throw Error('Invalid disposable runtime ownership');
 const guard=createOwnedSignalGuard(),stopped=[];
 for(let n=0;n<3;n++){
  const rows=await matching(root);if(!rows.length)return {stopped,remaining:[]};
  const captured=[];for(const row of rows){try{captured.push({...row,handle:guard.record(row.pid,{expectedStart:row.start})})}catch(e){if(processSnapshot().some(p=>p.pid===row.pid&&p.start===row.start&&p.state!=='Z'))throw e;}}
  for(const p of captured){try{guard.pid(p.handle,'SIGTERM')}catch(e){if(e.code!=='ESRCH')throw e;}}
  await new Promise(r=>setTimeout(r,500));
  for(const p of captured)if(processSnapshot().some(x=>x.pid===p.pid&&x.start===p.start&&x.state!=='Z'))try{guard.pid(p.handle,'SIGKILL')}catch(e){if(e.code!=='ESRCH')throw e;}
  stopped.push(...rows);await new Promise(r=>setTimeout(r,250));
 }
 const remaining=await matching(root);if(remaining.length)throw Error('Owned detached runtime processes remain');return {stopped,remaining};
}

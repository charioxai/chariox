// MP-08 / MP-10 / MP-11: interrupting the driver must settle its exact detached resource.
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtemp,readFile,rm} from 'node:fs/promises';import {join} from 'node:path';
import {spawnOwned,signalOwnedProcessGroup,processSnapshot} from './owned.mjs';
import {createOwnedSignalGuard} from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
for(const signal of ['SIGINT','SIGTERM'])for(const phase of ['before-launch','after-launch'])test('MP-08 / MP-10 / MP-11 '+signal+' '+phase+' cleans owned resources before exit',async()=>{
 if(!process.env.TMPDIR)throw Error('Disk TMPDIR required');const root=await mkdtemp(join(process.env.TMPDIR,'termination-'));let child,resource,guard=createOwnedSignalGuard();
 try{
  child=spawnOwned(process.execPath,[new URL('./termination-fixture.mjs',import.meta.url).pathname,root,phase],{stdio:'ignore',detached:true});
  let ready;for(let n=0;n<100;n++){try{ready=JSON.parse(await readFile(root+'/ready.json','utf8'));break;}catch{await sleep(50);}}
  assert(ready);assert(Number.isInteger(ready.driver)&&ready.driver>1);if(phase==='after-launch')assert(Number.isInteger(ready.resource)&&ready.resource>1);
  const driverRow=processSnapshot().find(p=>p.pid===ready.driver),resourceRow=processSnapshot().find(p=>p.pid===ready.resource);assert(driverRow);if(phase==='after-launch')assert(resourceRow);
  const driver=guard.record(ready.driver,{expectedStart:driverRow.start});if(resourceRow)resource=guard.record(ready.resource,{expectedStart:resourceRow.start,sessionLeader:true});
  guard.pid(driver,signal);
  for(let n=0;n<100&&child.exitCode===null&&child.signalCode===null;n++)await sleep(50);
  assert.equal(await readFile(root+'/cleaned','utf8'),'yes');
  if(resourceRow)assert(!processSnapshot().some(p=>p.pid===ready.resource&&p.start===resourceRow.start&&p.state!=='Z'));
 }finally{
  if(resource)try{guard.group(resource,'SIGKILL');}catch(e){if(e.code!=='ESRCH')throw e;}
  if(child&&child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGKILL');await rm(root,{recursive:true,force:true});
 }
});
test('MP-08 / MP-10 / MP-11 pending reservation is settled before termination exits',async()=>{
 const root=await mkdtemp(join(process.env.TMPDIR,'termination-pending-'));let child;
 try{
  child=spawnOwned(process.execPath,[new URL('./termination-acquisition-fixture.mjs',import.meta.url).pathname,root],{stdio:'ignore',detached:true});
  let ready;for(let n=0;n<100;n++){try{ready=JSON.parse(await readFile(root+'/ready.json','utf8'));break;}catch{await sleep(50);}}
  assert(ready&&Number.isInteger(ready.driver)&&ready.driver>1);const row=processSnapshot().find(p=>p.pid===ready.driver);assert(row);const guard=createOwnedSignalGuard();const handle=guard.record(ready.driver,{expectedStart:row.start});guard.pid(handle,'SIGTERM');
  for(let n=0;n<100;n++){try{if(await readFile(root+'/cleaned','utf8')==='yes')break;}catch{}await sleep(50);}
  assert.equal(await readFile(root+'/cleaned','utf8'),'yes');await assert.rejects(readFile(root+'/reservation'),{code:'ENOENT'});
 }finally{if(child&&child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGKILL');await rm(root,{recursive:true,force:true});}
});

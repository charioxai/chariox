// MP-08 / MP-10 / MP-11: supplementary cleanup isolation with actual owned processes.
import test from 'node:test';import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';
import {createOwnedSignalGuard,processSnapshot} from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs';
import {createOwnedRuntime,sweepOwnedRuntime} from './runtime-owned.mjs';
test('detached runtime cleanup preserves a different runtime and rejects an unowned root',async()=>{
 const parent=await mkdtemp(join(tmpdir(),'context-owned-')),root=parent+'/run-owned',other=parent+'/run-other',guard=createOwnedSignalGuard(),children=[];
 try{
  await createOwnedRuntime(root,{name:'owned',head:'0'.repeat(40)});await createOwnedRuntime(other,{name:'other',head:'0'.repeat(40)});await rm(other+'/context-switch-owned.json');await writeFile(other+'/keep.txt','untouched');await assert.rejects(createOwnedRuntime(other,{name:'other',head:'0'.repeat(40)}),{code:'EEXIST'});assert.equal(await readFile(other+'/keep.txt','utf8'),'untouched');
  for(const home of [root,other]){const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{env:{PATH:process.env.PATH,HOME:home+'/os-home'},stdio:'ignore'});if(!Number.isInteger(child.pid)||child.pid<=1)throw Error('Unsafe test PID');children.push({child,handle:guard.record(child.pid)});}
  await new Promise(r=>setTimeout(r,200));const result=await sweepOwnedRuntime(root);assert.equal(result.remaining.length,0);assert(result.stopped.some(p=>p.pid===children[0].child.pid));assert(processSnapshot().some(p=>p.pid===children[1].child.pid&&p.state!=='Z'));await assert.rejects(sweepOwnedRuntime(other));
 }finally{for(const {child,handle}of children)if(child.exitCode===null&&child.signalCode===null)try{guard.pid(handle,'SIGKILL')}catch(e){if(e.code!=='ESRCH')throw e;}await Promise.all(children.map(({child})=>child.exitCode!==null||child.signalCode!==null?null:new Promise(r=>child.once('exit',r))));await rm(parent,{recursive:true,force:true});}
});

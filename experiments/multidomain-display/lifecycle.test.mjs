// MD-DISPLAY-02/03: failure receipts and signal safety are correctness gates.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm,symlink,readdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {stopGroup} from './runtime.mjs';
const runtime=new URL('./runtime.mjs',import.meta.url).href;
async function command(args,env){const child=spawn(process.execPath,args,{env:{...process.env,...env},stdio:['ignore','pipe','pipe']});let log='';for(const s of [child.stdout,child.stderr])s.on('data',b=>log+=b);const code=await new Promise((r,j)=>{child.on('error',j);child.on('close',r)});return{code,log}}
test('MD-DISPLAY-02 invalid/group-unowned PIDs never reach process.kill',async()=>{
 const original=process.kill,calls=[];process.kill=(...args)=>calls.push(args);
 try{for(const pid of [undefined,null,NaN,0,1,-1,-20,1.5,process.pid])await stopGroup({pid});assert.deepEqual(calls,[])}finally{process.kill=original}
});
test('MD-DISPLAY-02 missing Chrome rejects through caller finally and removes profile',async()=>{
 const before=(await readdir(tmpdir())).filter(n=>n.startsWith('chariox-md-display-missing-test-'));
 const output=await mkdtemp(path.join(tmpdir(),'md-display-launch-test-'));
 try{const r=await command(['--input-type=module','-e',`import {headedBrowser} from ${JSON.stringify(runtime)};import {writeFile} from 'node:fs/promises';try{await headedBrowser({},process.env.TEST_OUTPUT,'missing-test',':0')}catch(e){await writeFile(process.env.TEST_OUTPUT+'/caught',e.code||e.message)}`],{MD_CHROME:'/missing-md-display-chrome',TEST_OUTPUT:output});assert.equal(r.code,0,r.log);assert.match(await readFile(path.join(output,'caught'),'utf8'),/ENOENT/);assert.deepEqual((await readdir(tmpdir())).filter(n=>n.startsWith('chariox-md-display-missing-test-')),before)}finally{await rm(output,{recursive:true,force:true})}
});
test('MD-DISPLAY-02 missing Xvfb rejects through caller finally',async()=>{
 const r=await command(['--input-type=module','-e',`import {display} from ${JSON.stringify(runtime)};try{await display()}catch(e){console.log('caught',e.code)}`],{PATH:'/missing-md-display-bin'});assert.equal(r.code,0,r.log);assert.match(r.log,/caught ENOENT/)
});
test('MD-DISPLAY-02/03 campaign aggregates failed children and retains all receipts',async()=>{
 const output=await mkdtemp('/root/.codex/evidence/browser-resume-20260930/display/review-recovery/campaign-test-');
 try{await writeFile(path.join(output,'node'),'#!/bin/sh\nexit 124\n',{mode:0o700});const r=await command(['experiments/multidomain-display/campaign.mjs'],{PATH:output+':'+process.env.PATH,MD_CAMPAIGN_OUTPUT:output,MD_METHODS:'h264',MD_RATES:'500000,1000000'});assert.equal(r.code,1,r.log);const m=JSON.parse(await readFile(path.join(output,'campaign.json')));assert.equal(m.runs.length,2);assert.ok(m.runs.every(r=>r.exit_code===124))}finally{await rm(output,{recursive:true,force:true})}
});

test('MD-DISPLAY-02 Xvfb readiness timeout cleans the acquired child',async()=>{
 const {display}=await import('./runtime.mjs');const output=await mkdtemp(path.join(tmpdir(),'md-display-timeout-test-'));const pidFile=path.join(output,'pid');
 try{await assert.rejects(display({executable:process.execPath,args:['-e',`require('fs').writeFileSync(${JSON.stringify(pidFile)},String(process.pid));setInterval(()=>{},1000)`],timeout:300}),/timeout: Xvfb/);const pid=Number(await readFile(pidFile));await assert.rejects(readFile(`/proc/${pid}/stat`),{code:'ENOENT'})}finally{await rm(output,{recursive:true,force:true})}
});
test('MD-DISPLAY-02 signalChild rejects invalid and unregistered identities',async()=>{
 const {signalChild}=await import('./owned-process.mjs'),original=process.kill,calls=[];process.kill=(...args)=>calls.push(args);
 try{for(const pid of [undefined,null,NaN,0,1,-1,process.pid])await signalChild({pid});assert.deepEqual(calls,[])}finally{process.kill=original}
});
test('MD-DISPLAY-02 verified owned child and descendant groups settle',async()=>{
 const {launchOwned,rememberGroup}=await import('./owned-process.mjs');const child=await launchOwned(process.execPath,['-e',"const {spawn}=require('child_process');const c=spawn(process.execPath,['-e','setInterval(()=>{},1000)']);console.log(c.pid);setInterval(()=>{},1000)"],{detached:true,stdio:['ignore','pipe','ignore']});
 try{const pid=Number(await new Promise(r=>child.stdout.once('data',b=>r(b.toString()))));const members=await rememberGroup(child);assert.ok(members.some(p=>p.pid===pid));await stopGroup(child);const {resources}=await import('./runtime.mjs');assert.deepEqual((await resources([child.pid])).processes.filter(p=>p.group===child.pid&&p.state!=='Z'),[])}finally{await stopGroup(child)}
});
test('MD-DISPLAY-02/03 campaign distinguishes success, failures, signals and launch errors',async()=>{
 for(const [body,exit] of [['process.exit(0)',0],['process.exit(1)',1],['process.exit(130)',1],["if(!Number.isSafeInteger(process.pid)||process.pid<=1)throw Error('unsafe');process.kill(process.pid,'SIGTERM')",1],['bad interpreter',1]]){
  const output=await mkdtemp('/root/.codex/evidence/browser-resume-20260930/display/review-recovery/campaign-exit-');
  try{await symlink('/usr/bin/git',path.join(output,'git'));await writeFile(path.join(output,'node'),body==='bad interpreter'?'#!/missing-md-display-interpreter\n':`#!${process.execPath}\n${body}\n`,{mode:0o700});const r=await command(['experiments/multidomain-display/campaign.mjs'],{PATH:body==='bad interpreter'?output:output+':'+process.env.PATH,MD_CAMPAIGN_OUTPUT:output,MD_METHODS:'h264',MD_RATES:'1000000'});assert.equal(r.code,exit,r.log);const m=JSON.parse(await readFile(path.join(output,'campaign.json')));assert.equal(m.exit_code,exit);assert.equal(m.runs.length,1);if(body==='bad interpreter')assert.match(m.runs[0].launch_error,/ENOENT/);if(body.includes('process.kill'))assert.equal(m.runs[0].signal,'SIGTERM')}finally{await rm(output,{recursive:true,force:true})}
 }
});
test('MD-DISPLAY-02/03 interrupted campaign exits 130 and stops admitting cases',async()=>{
 const {launchOwned,signalChild,waitChild}=await import('./owned-process.mjs');const output=await mkdtemp('/root/.codex/evidence/browser-resume-20260930/display/review-recovery/campaign-interrupt-');let child;
 try{
  await writeFile(path.join(output,'node'),`#!${process.execPath}\nconsole.log('MD-DISPLAY test child ready');setInterval(()=>{},1000)\n`,{mode:0o700});
  child=await launchOwned(process.execPath,['experiments/multidomain-display/campaign.mjs'],{env:{...process.env,PATH:output+':'+process.env.PATH,MD_CAMPAIGN_OUTPUT:output,MD_METHODS:'h264',MD_RATES:'500000,1000000'},stdio:['ignore','pipe','pipe']});
  await new Promise(r=>child.stdout.once('data',r));await signalChild(child,'SIGTERM');assert.equal((await waitChild(child)).code,130);const m=JSON.parse(await readFile(path.join(output,'campaign.json')));assert.equal(m.exit_code,130);assert.equal(m.runs.length,1);assert.equal(m.interruption,'SIGTERM');assert.equal(m.runs[0].signal,'SIGTERM');
 }finally{await signalChild(child,'SIGTERM');await rm(output,{recursive:true,force:true})}
});

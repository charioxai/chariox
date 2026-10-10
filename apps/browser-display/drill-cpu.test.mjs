import {mkdtemp,symlink,rm} from 'node:fs/promises';
import {launchOwned,stopGroup} from './drill-owned-process.mjs';

// MP-10: retain departed processes and reject PID reuse as an accounting base.
import test from 'node:test';import assert from 'node:assert/strict';
import {CpuCounter,cpuSpan,CpuSampler} from './drill-cpu.mjs';
test('MP-10 CPU retains exited encoder consumption across spans and PID reuse',()=>{
 const c=new CpuCounter(100),before={at_ms:0,cpu_seconds:c.observe([{pid:42,start:'1',ticks:100,role:'pipeline'}])};
 c.observe([{pid:42,start:'1',ticks:125,role:'pipeline'}]);c.observe([]);
 const after={at_ms:1000,cpu_seconds:c.observe([{pid:42,start:'2',ticks:50,role:'source'}])};
 assert.deepEqual(cpuSpan(before,after).cores,{source:.5,pipeline:.25,viewer:0,harness:0});
});

// MP-10: assigning exec CPU to source must never reclassify earlier pipeline CPU.
test('MP-10 CPU role transitions retain earlier ticks and produce nonnegative spans',()=>{
 const c=new CpuCounter(100);c.observe([{pid:42,start:'1',ticks:25,role:'pipeline'}]);
 const before={at_ms:0,cpu_seconds:c.totals()};
 const after={at_ms:1000,cpu_seconds:c.observe([{pid:42,start:'1',ticks:75,role:'source'}])};
 assert.deepEqual(cpuSpan(before,after).cores,{source:.5,pipeline:0,viewer:0,harness:0});
 assert.deepEqual(c.totals(),{source:.5,pipeline:.25,viewer:0,harness:0});
});

test('MP-10 source browser exec leaves its launcher pipeline classification',async()=>{
 const root=await mkdtemp('/tmp/chariox-display25-cpu-');let child;
 try{
  await symlink(process.execPath,root+'/chrome');
  child=await launchOwned('/bin/sh',['-c','read gate; exec "$1/chrome" -e "setTimeout(()=>{},5000)"','_',root],{detached:true,stdio:['pipe','ignore','ignore']});
  const sampler=new CpuSampler({root,viewerRoot:root+'/viewer'});
  const before=await sampler.sample();assert.equal(before.processes.find(p=>p.pid===child.pid)?.role,'pipeline');
  child.stdin.write('go\n');let after;
  for(let i=0;i<5;i++){await new Promise(r=>setTimeout(r,50));after=await sampler.sample();if(after.processes.find(p=>p.pid===child.pid)?.role==='source')break;}
  assert.equal(after.processes.find(p=>p.pid===child.pid)?.role,'source');
 }finally{await stopGroup(child);await rm(root,{recursive:true,force:true});}
});

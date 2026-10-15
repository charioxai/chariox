// MP-08/MP-10/MP-11: bounded timing records must survive batching/shutdown.
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,existsSync,rmSync} from 'node:fs';import {tmpdir} from 'node:os';import path from 'node:path';
import {displayTiming,timedHostRequests} from './kernel-browser-timing.mjs';
test('MP-08/MP-10 native diagnostics batch without losing order or final shutdown records',()=>{
 const root=mkdtempSync(path.join(tmpdir(),'chariox-mp24-timing-')),old=process.env.CHARIOX_BROWSER_DISPLAY_TIMING;
 const timing=displayTiming(root),file=path.join(root,'display-timing.jsonl');
 try{
  delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;timing('disabled',0,1);assert(!existsSync(file));
  process.env.CHARIOX_BROWSER_DISPLAY_TIMING='1';timing('capture',1,2);timing.batch([['convert',2,3],['encode',3,5]]);assert(!existsSync(file));
  for(let i=0;i<600;i++)timing('control',i,i+1);
  assert(existsSync(file),'bounded accumulation flushes before growing');timing.flush();
  const records=readFileSync(file,'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(records.length,603);assert.deepEqual(records.slice(0,3).map(r=>[r.stage,r.duration_ms]),[['capture',1],['convert',1],['encode',2]]);
  assert.deepEqual(records.slice(3).map(r=>r.started_ms),Array.from({length:600},(_,i)=>i));
 }finally{timing.flush();if(old===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;else process.env.CHARIOX_BROWSER_DISPLAY_TIMING=old;rmSync(root,{recursive:true,force:true})}
});
test('MP-08/MP-10 input/frame join records keep finite numbers only',()=>{
 const root=mkdtempSync(path.join(tmpdir(),'chariox-mp24-timing-')),old=process.env.CHARIOX_BROWSER_DISPLAY_TIMING;
 const timing=displayTiming(root),file=path.join(root,'display-timing.jsonl');
 try{
  delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;timing.event('frame_out',{at:1,sequence:1});timing.flush();assert(!existsSync(file));
  process.env.CHARIOX_BROWSER_DISPLAY_TIMING='1';
  timing.event('frame_out',{at:5,sequence:7,kind:1,published_ms:6,encoded_ms:undefined,text:'page data'});timing.flush();
  const [record]=readFileSync(file,'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(record.stage,'frame_out');assert.equal(record.started_ms,5);assert.equal(record.sequence,7);assert.equal(record.published_ms,6);
  assert(!('text' in record)&&!('encoded_ms' in record)&&!('at' in record));
 }finally{timing.flush();if(old===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING;else process.env.CHARIOX_BROWSER_DISPLAY_TIMING=old;rmSync(root,{recursive:true,force:true})}
});

test('MP-08/MP-10/MP-11 scheduler attribution records only input waiting time',()=>{
 const events=[],handled=[],host={timing:(...args)=>events.push(args),handle:(...args)=>(handled.push(args),'result')};
 const adapter=timedHostRequests(host,()=>({kind:'barrier'}));
 const input={method:'host.browser',params:{op:'input',input:{kind:'text',text:'must never be logged'},document_id:'private-binding'}};
 const native={method:'host.computer',params:{op:'input',input:{kind:'text',text:'must never be logged'},surface_id:'private-binding'}};
 const read={method:'host.browser',params:{op:'state'}};
 assert.deepEqual(adapter.scheduleRequest(input),{kind:'barrier'});adapter.scheduleRequest(read);adapter.scheduleRequest(native);
 assert.equal(adapter.handleRequest(input,{signal:null}),'result');adapter.handleRequest(read,{});adapter.handleRequest(native,{});
 assert.equal(events.length,2);for(const event of events){assert.equal(event.length,2);assert.equal(event[0],'host_input_queue');assert(Number.isFinite(event[1]));}assert.equal(events[0].length,2);assert.equal(events[0][0],'host_input_queue');assert(Number.isFinite(events[0][1]));
 assert.equal(handled[0][0],input);assert.equal(handled[1][0],read);
});

test('MP-08/MP-10/MP-11 region target attribution spans completion without exposing the request',async()=>{
 const events=[],gate=Promise.withResolvers();
 const query={method:'host.browser',params:{op:'display_input_target',binding_id:'private-binding',input:{text:'private-input'}}};
 const host={timing:(...args)=>events.push(args),handle:()=>gate.promise};
 const adapter=timedHostRequests(host,()=>({kind:'barrier'}));
 adapter.scheduleRequest(query);const result=adapter.handleRequest(query,{});
 assert.deepEqual(events.map(e=>e[0]),['host_input_target_queue']);
 gate.resolve('target');assert.equal(await result,'target');
 assert.deepEqual(events.map(e=>e[0]),['host_input_target_queue','host_input_target']);
 for(const e of events){assert.equal(e.length,2);assert(Number.isFinite(e[1]))}
 assert(!JSON.stringify(events).includes('private-'));
});

test('MP-08/MP-10/MP-11 target attribution preserves sync results and failures',async()=>{
 const query={method:'host.browser',params:{op:'display_input_target'}},events=[];
 const failure=Error('private-failure');
 const adapter=handle=>timedHostRequests({timing:(...args)=>events.push(args),handle},()=>null);
 assert.equal(adapter(()=>7).handleRequest(query,{}),7);
 assert.throws(()=>adapter(()=>{throw failure}).handleRequest(query,{}),error=>error===failure);
 await assert.rejects(adapter(()=>Promise.reject(failure)).handleRequest(query,{}),error=>error===failure);
 assert.equal(events.length,3);for(const e of events){assert.equal(e[0],'host_input_target');assert.equal(e.length,2);assert(Number.isFinite(e[1]))}
 assert(!JSON.stringify(events).includes('private-'));
});

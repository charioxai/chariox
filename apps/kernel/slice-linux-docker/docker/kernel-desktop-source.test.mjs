// MP-08 / MP-11: native desktop readbacks are masked from bracketing protection snapshots.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DesktopSource, nativeDesktopWorker } from './kernel-desktop-source.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
import { setTimeout as delay } from 'node:timers/promises';
const binding={width:1280,height:800,ownedProcesses:async()=>[],browserProcesses:async()=>[]};
function fixture({mask=false}={}){
  const source=new DesktopSource(binding,{values:[],targets:[],unknown:false},{native:{executable:'/worker',root:'/packets'}});
  const notices=[],published=[],snapshots=[];let clock=1000,released=0;
  source.current={processes:[],browser_processes:[],browser_protection:null,serial:0,mask};
  source.worker={notify:command=>notices.push(command)};
  source.subscribe(sample=>published.push(sample));
  // Each snapshot takes 10 ms of wall clock; the test scripts their digests.
  source.measure=async scope=>{
    const start=clock;clock+=10;const [digest,masks]=snapshots.shift()??['same',[[0,0,10,10]]];
    const snapshot={scope,digest,masks,start,end:clock+1};source.history=[...(source.history??[]),snapshot];return snapshot;
  };
  const raw=(serial,captured_ms)=>({serial,captured_ms,width:1280,height:800,pixels:Buffer.alloc(1280*800*4,200),retain(){},release(){released++}});
  return {source,notices,published,snapshots,raw,advance:ms=>{clock+=ms},now:()=>clock,released:()=>released};
}
test('MP-11 a native readback with no preceding snapshot is dropped and read again',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  assert.equal(f.published.length,0);assert.equal(f.released(),1);assert.deepEqual(f.notices,[{refresh:true}]);
});
test('MP-08/MP-11 a readback between two equal snapshots carries exactly their masks',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.source.next=f.raw(2,f.now()+2);await f.source.protect();
  assert.equal(f.published.length,1);
  assert.deepEqual(f.published[0].raw[displayMaskRegions],[{x:0,y:0,width:10,height:10}]);
  assert.equal(f.published[0].raw.nativeEncode,undefined,'the fixture raw has no worker');
});
test('MP-11 a protection change across the readback masks the whole desktop',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.snapshots.push(['changed',[]]);
  f.source.next=f.raw(2,f.now()+2);await f.source.protect();
  assert.deepEqual(f.published[0].raw[displayMaskRegions],[{x:0,y:0,width:1280,height:800}]);
  assert.deepEqual(f.notices,[{refresh:true},{refresh:true}],'a whole-masked frame asks for a fresh readback (no sticky black on a still desktop)');
});
test('MP-11 unavailable or protected snapshots mask the whole desktop',async()=>{
  const f=fixture();
  f.snapshots.push(['same',null],['same',null]);
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.source.next=f.raw(2,f.now()+2);await f.source.protect();
  assert.deepEqual(f.published[0].raw[displayMaskRegions],[{x:0,y:0,width:1280,height:800}]);
});
test('MP-11 an unobserved gap above 200 ms before the readback drops it',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.advance(500);f.source.next=f.raw(2,f.now()-100);await f.source.protect();
  assert.equal(f.published.length,0);assert.equal(f.notices.length,2);
});
test('MP-11 a new process or protection scope retires earlier snapshots',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.source.current={...f.source.current,processes:[{pid:7,started:'1'}]};
  f.source.next=f.raw(2,f.now()+2);await f.source.protect();
  assert.deepEqual(f.published[0].raw[displayMaskRegions],[{x:0,y:0,width:1280,height:800}]);
});
test('MP-11 an unknown policy masks every native readback without snapshots',async()=>{
  const f=fixture({mask:true});
  f.source.measure=async()=>{throw Error('no snapshot under a masking policy')};
  f.source.next=f.raw(1,f.now());await f.source.protect();
  assert.deepEqual(f.published[0].raw[displayMaskRegions],[{x:0,y:0,width:1280,height:800}]);
});
test('MP-08 the native worker is used only with an absolute worker and packet root',()=>{
  assert.equal(nativeDesktopWorker({}),null);
  assert.equal(nativeDesktopWorker({CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER:'/k'}),null);
  assert.equal(nativeDesktopWorker({CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER:'k',CHARIOX_BROWSER_DISPLAY_PACKET_ROOT:'/p'}),null);
  assert.deepEqual(nativeDesktopWorker({CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER:'/k',CHARIOX_BROWSER_DISPLAY_PACKET_ROOT:'/p'}),{executable:'/k',root:'/p'});
});
test('MP-08/MP-11 a process run-state change keeps the protection scope; a new process replaces it',async()=>{
  let processes=[{pid:9,parent:1,started:'7',state:'R'},{pid:8,parent:1,started:'6',state:'S'}];
  const source=new DesktopSource({...binding,ownedProcesses:async()=>processes.map(item=>({...item}))},{values:[],targets:[],unknown:false},{native:{executable:'/worker',root:'/packets'}});
  const notices=[];source.worker={notify:command=>notices.push(command)};
  source.current=await source.scope();
  const first=source.current;
  assert.deepEqual(first.processes,[{pid:8,parent:1,started:'6'},{pid:9,parent:1,started:'7'}],'identity only, canonical order');
  processes=[{pid:8,parent:1,started:'6',state:'R'},{pid:9,parent:1,started:'7',state:'D'}];
  await source.wake();
  assert.equal(source.current,first,'run state is not process identity');
  processes=[...processes,{pid:10,parent:9,started:'8',state:'S'}];
  await source.wake();
  assert.notEqual(source.current,first);assert.equal(source.current.processes.length,3);
  assert.equal(notices.length,2);
});
test('MP-08/MP-11 idle snapshots give the first readback after a quiet desktop its preceding observation',async()=>{
  const f=fixture();
  f.source.next=f.raw(1,f.now());await f.source.protect();
  f.source.next=f.raw(2,f.now()+2);await f.source.protect();
  assert.equal(f.published.length,1);
  // A quiet second: heartbeat snapshots keep every gap below 200 ms.
  for(let n=0;n<10;n++){f.advance(90);await f.source.observeIdle();}
  f.source.next=f.raw(3,f.now()+2);await f.source.protect();
  assert.equal(f.published.length,2,'bound without a drop/refresh round trip');
  assert.deepEqual(f.notices,[{refresh:true}]);
  // Never while a readback waits or under a masking policy.
  const calls=f.source.history.length;f.source.next=f.raw(4,f.now());await f.source.observeIdle();
  assert.equal(f.source.history.length,calls);f.source.next=null;
  f.source.current={...f.source.current,mask:true};await f.source.observeIdle();assert.equal(f.source.history.length,calls);
});
const until=async(predicate,ms=2000)=>{for(const end=Date.now()+ms;!predicate();){if(Date.now()>end)throw Error('condition timeout');await new Promise(resolve=>setTimeout(resolve,2));}};
function gated(f){
  const steps=[];
  f.source.gate={protectionSerial:1,protection:null,step:()=>new Promise(resolve=>{const started=Date.now();steps.push({started,finish:(verified=1,changed=false)=>resolve({started,verified,changed})});})};
  const frame=(serial,captured)=>({raw:{...f.raw(serial,captured),serial},serial,width:1280,height:800,motion:true});
  return {steps,frame};
}
test('MP-08/MP-11 a gate step starts when a readback arrives and verifies a frame bound during it',async()=>{
  const f=fixture(),{steps,frame}=gated(f);
  f.source.next=f.raw(1,Date.now()-5);
  const loop=f.source.verify();
  try{
    await until(()=>steps.length===1);
    f.source.next=null;f.source.offer(frame(1,Date.now()-5),1,Date.now()-5);
    assert.equal(f.published.length,0,'never before a step that began after the capture completes');
    steps[0].finish();
    await until(()=>f.published.length===1);
  }finally{f.source.closed=true;steps.at(-1)?.finish();await loop;}
});
test('MP-08/MP-11 each step publishes the newest frame captured before it began; later frames wait, retired serials never publish',async()=>{
  const f=fixture(),{steps,frame}=gated(f);
  const before=Date.now()-5;
  f.source.offer(frame(1,before-3),1,before-3);f.source.offer(frame(2,before),1,before);
  const loop=f.source.verify();
  try{
    await until(()=>steps.length===1);
    const start=steps[0].started;
    f.source.offer(frame(3,start+1),1,start+1);f.source.offer(frame(4,start+2),1,start+2);
    steps[0].finish();
    await until(()=>f.published.length===1);
    assert.equal(f.published[0].serial,2);
    await until(()=>steps.length===2);
    steps[1].finish();
    await until(()=>f.published.length===2);
    assert.equal(f.published[1].serial,4,'the frame captured during the first step is verified by the second');
    f.source.offer(frame(5,Date.now()),1,Date.now()+1);
    await until(()=>steps.length===3);
    steps[2].finish(null,true);await delay(20);
    assert.equal(f.published.length,2,'a changed measurement verifies nothing');
  }finally{f.source.closed=true;steps.at(-1)?.finish();await loop;}
});
test('MP-08/MP-11 a withheld kernel-browser window keeps the gate re-measuring on a still desktop',async()=>{
  const f=fixture(),{steps}=gated(f);
  f.source.withheld=true;
  const loop=f.source.verify();
  try{
    await until(()=>steps.length===1,1000);steps[0].finish(1,false);
    await until(()=>steps.length===2,1000);
    f.source.withheld=false;steps[1].finish(1,false);
    await delay(400);assert.equal(steps.length,2,'a bound desktop with nothing pending does not measure');
  }finally{f.source.closed=true;steps.at(-1)?.finish();await loop;}
});
test('MP-08/MP-11 Vault values never mask the whole desktop; the oracle gets them for best-effort boxes',async()=>{
  const policy={values:['synthetic-vault-value'],targets:[{kind:'native',target:{focus_window:1,active_window:2}}],unknown:false};
  const source=new DesktopSource({...binding},policy,{native:{executable:'/worker',root:'/packets'}});
  const scope=await source.scope();
  assert.equal(scope.mask,false,'owner 2026-10-09: no blackout under Vault values');
  assert.deepEqual(scope.values,['synthetic-vault-value']);
  const written=[];source.oracle={stdin:{write:line=>written.push(JSON.parse(line))}};
  source.answers={next:async()=>({done:false,value:JSON.stringify({digest:'d',masks:[],state:[1,1,0,0]})})};
  await source.measure(scope);
  assert.deepEqual(written[0].values,['synthetic-vault-value']);
});

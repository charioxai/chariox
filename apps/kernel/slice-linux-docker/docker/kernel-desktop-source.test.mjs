// MP-08 / MP-11: native desktop readbacks are masked from bracketing protection snapshots.
import test from 'node:test';
import assert from 'node:assert/strict';
import { DesktopSource, nativeDesktopWorker } from './kernel-desktop-source.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
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
test('MP-11 a Vault or unknown policy masks every native readback without snapshots',async()=>{
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

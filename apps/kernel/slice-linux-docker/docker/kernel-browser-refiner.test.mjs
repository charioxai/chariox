// MD-DISPLAY-02/04: exact work cannot block input or commit an overtaken source.
import test from 'node:test';import assert from 'node:assert/strict';import {NativeRefiner} from './kernel-browser-refiner.mjs';
import {PixelWorker} from './kernel-browser-pixel-worker.mjs';
import {decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
const binding={document:'d',policy:{},epoch:1,serial:1,scale:2};
test('MP-08/MP-10/MP-11 admitted native exact repair never acquires CDP screenshot lane',async()=>{
 let retained=0;const data=Buffer.from([3,2,1,0,0,0,0,0]);
 const raw={width:2,height:1,pixels:data,retain:()=>retained++,release:()=>retained--,[displayMaskRegions]:[{x:1,y:0,width:1,height:1}]};
 const sample={raw,width:2,height:1,tab_id:'t',document_id:'d',motion:true};
 const r=new NativeRefiner(async b=>{assert.equal(b.sample,sample);return sample;},{now:()=>400,pixels:new PixelWorker()});
 try{
  r.request({...binding,native:true,sample},0,()=>true);await r.active;
  assert.equal(r.failure,undefined);assert.equal(retained,0);assert.equal(r.latest.motion,false);
  assert.deepEqual([...r.latest.pixels.pixels],[1,2,3,255,0,0,0,255]);
  assert.deepEqual(r.latest[displayMaskRegions],raw[displayMaskRegions]);
  assert.equal(typeof r.latest.data_base64,'string','complete exact PNG must be prepared off the host input loop');
  assert.deepEqual(decodePng(r.latest.data_base64).pixels,r.latest.pixels.pixels);
 }finally{await r.close()}
});
test('MD-DISPLAY native read is asynchronous, bounded and invalidated before commit',async()=>{let release,captures=0,now=200;const r=new NativeRefiner(()=>{captures++;return new Promise(ok=>release=ok)},{quietMs:80,now:()=>now,pixels:{async run(){return {width:1,height:1,pixels:Buffer.alloc(4)}},async close(){}}});assert.equal(r.request(binding,0,()=>true),null);assert.equal(r.request(binding,0,()=>true),null);assert.equal(captures,1);r.request({...binding,epoch:2},0,()=>true);release({data_base64:'x'});await r.active;assert.equal(r.latest,null);assert.equal(r.request({...binding,epoch:2},0,()=>true),null);release({data_base64:'x'});await r.active;assert.equal(r.latest.settled_verified,true);await r.close()});
test('MD-DISPLAY native verification errors follow awaited failure path',async()=>{const r=new NativeRefiner(async()=>{throw Error('fixture')},{quietMs:80,now:()=>200,pixels:{async close(){}}});r.request(binding,0,()=>true);await r.active;assert.throws(()=>r.request(binding,0,()=>true),/verification failed/);await r.close()});
test('MD-DISPLAY close waits for owned verification and never publishes its result',async()=>{let release,closed=false;const r=new NativeRefiner(()=>new Promise(ok=>release=ok),{quietMs:80,now:()=>200,pixels:{async run(){return {}},async close(){closed=true}}});r.request(binding,0,()=>true);const closing=r.close();await Promise.resolve();assert.equal(closed,false);release({data_base64:'x'});await closing;assert.equal(closed,true);assert.equal(r.latest,null)});

test('MD-DISPLAY unchanged JPEG serial still rechecks fine PNG detail at deadline/input epoch',async()=>{
 let now=200,value=1,reads=0;
 const source={},initial={...binding,source};
 const r=new NativeRefiner(async()=>{reads++;return{data_base64:String(value)}},{quietMs:80,now:()=>now,pixels:{run:async(_op,{data})=>({pixels:Buffer.from(data)}),close:async()=>{}}});
 try{
  r.request(initial,0,()=>true);await r.active;assert.equal(r.latest.pixels.pixels.toString(),'1');
  value=2;now=451;r.request(initial,0,()=>true);await r.active;
  assert.equal(reads,2);assert.equal(r.latest.pixels.pixels.toString(),'2');
  value=3;r.request({...initial,epoch:2},0,()=>true);await r.active;
  assert.equal(r.latest.pixels.pixels.toString(),'3');
  r.request({...initial,source:{}},0,()=>true);assert.equal(r.latest,null,'same serial on a different source cannot reuse exact pixels');await r.active;
 }finally{await r.close()}
});

test('MD-DISPLAY cached repair RGB avoids tile re-encoding but never skips deadline capture',async()=>{
 let now=200,reads=0,preparations=0,value=1;
 const r=new NativeRefiner(async()=>{reads++;return{data_base64:'png'}},{quietMs:80,now:()=>now,prepareTiles:true,pixels:{
  async run(kind){if(kind==='decode')return{width:1,height:1,pixels:Buffer.from([value,2,3,255])};preparations++;return['prepared-'+value]},async close(){}}});
 try{
  r.request(binding,0,()=>true);await r.active;assert.equal(preparations,1);
  now=451;r.request(binding,0,()=>true);await r.active;assert.equal(reads,2);assert.equal(preparations,1);
  value=2;now=702;r.request(binding,0,()=>true);await r.active;assert.equal(reads,3);assert.equal(preparations,2);
 }finally{await r.close()}
});

test('MD-DISPLAY full repair prepares off the credit path and stale preparation cannot publish',async()=>{
 let release,started,reads=0,preparations=0;
 const preparing=new Promise(resolve=>started=resolve);
 const r=new NativeRefiner(async()=>{reads++;return{data_base64:'png'}},{quietMs:80,now:()=>200,prepareTiles:true,pixels:{
  async run(kind){if(kind==='decode')return{width:1,height:1,pixels:Buffer.from([1,2,3,255])};
   preparations++;started();return new Promise(resolve=>release=resolve)},async close(){}}});
 try{
  assert.equal(r.request(binding,0,()=>true),null);await Promise.race([preparing,r.active]);
  assert.equal(preparations,1,'full tiles must be prepared asynchronously before credit selection');
  assert.equal(r.latest,null);
  assert.equal(r.request({...binding,epoch:2},0,()=>true),null);
  release([{x:0,y:0,width:1,height:1,data_base64:'tile'}]);await r.active;
  assert.equal(r.latest,null,'old binding cannot publish a prepared repair');
  r.request({...binding,epoch:2},0,()=>true);
  while(preparations<2)await new Promise(resolve=>setImmediate(resolve));
  release([{x:0,y:0,width:1,height:1,data_base64:'new-tile'}]);await r.active;
  assert.equal(r.latest.repair_tiles[0].data_base64,'new-tile');assert.equal(reads,2);
 }finally{release?.([]);await r.close()}
});

test('MP-08/MP-10 exact capture waits through a normal typing cadence',async()=>{
 let now=100,reads=0;const r=new NativeRefiner(async()=>{reads++;return{data_base64:'png'}},{now:()=>now,pixels:{run:async()=>({}),close:async()=>{}}});
 try{for(let at=100;at<=1000;at+=100){now=at;r.request({...binding,epoch:at},at,()=>true)}assert.equal(reads,0);now=1299;r.request({...binding,epoch:1000},1000,()=>true);assert.equal(reads,0);now=1300;r.request({...binding,epoch:1000},1000,()=>true);await r.active;assert.equal(reads,1)}finally{await r.close()}
});

test('MP-08/MP-11 native exact byte serial settles once; lossy fallback still verifies hidden changes',async()=>{
 let now=300,captures=0,pixel='one';
 const r=new NativeRefiner(async()=>({data_base64:pixel}),{now:()=>now,pixels:{run:async(op,{data})=>({pixels:Buffer.from(data),width:1,height:1}),close:async()=>{}}});
 const binding={source:{},document:'d',policy:{},epoch:0,serial:1,native:true};
 try{
  const capture=r.capture;r.capture=async()=>{captures++;return capture()};
  r.request(binding,0,()=>true);await r.active;assert.equal(captures,1);
  now=3000;r.request(binding,0,()=>true);await r.active;assert.equal(captures,1,'native exact bytes and serial already attest this unchanged source');
  r.request({...binding,serial:2},0,()=>true);await r.active;assert.equal(captures,2);
  const fallback={...binding,native:false};r.request(fallback,0,()=>true);await r.active;
  pixel='hidden RGB';now+=300;r.request(fallback,0,()=>true);await r.active;
  assert.equal(captures,4);assert.equal(r.latest.pixels.pixels.toString(),'hidden RGB','same lossy serial cannot suppress exact deadline verification');
 }finally{await r.close()}
});

// MP-08/MP-10: native byte serials need no delayed CDP verification.
test('MP-08/MP-10 native exact preparation starts within50ms of the final byte change',async()=>{
 let now=49,reads=0;
 const r=new NativeRefiner(async()=>{reads++;return {data_base64:'png'}},{now:()=>now,pixels:{run:async()=>({}),close:async()=>{}}});
 try{const native={...binding,native:true};r.request(native,0,()=>true);assert.equal(reads,0);now=50;r.request(native,0,()=>true);await r.active;assert.equal(reads,1)}finally{await r.close()}
});

test('MP-08/MP-10 an exact PNG within the repair budget skips redundant tile preparation',async()=>{
 let tiles=0;const r=new NativeRefiner(async()=>({data_base64:'small exact PNG'}),{now:()=>400,prepareTiles:true,pixels:{run:async kind=>{if(kind==='tiles')tiles++;return {pixels:Buffer.alloc(4),width:1,height:1}},close:async()=>{}}});
 try{r.request({...binding,repairLimit:1000},0,()=>true);await r.active;assert.equal(tiles,0);assert.equal(r.latest.settled_verified,true);assert.equal(r.latest.data_base64,'small exact PNG')}finally{await r.close()}
});

// MP-08/MP-10: 30Hz content is still moving at the old 30ms deadline.
test('MP-08/MP-10 native quiet deadline avoids 30Hz motion reads',async()=>{
 let now=33,reads=0;
 const r=new NativeRefiner(async()=>{reads++;return{data_base64:'png'}},{now:()=>now,pixels:{run:async()=>({}),close:async()=>{}}});
 try{r.request({...binding,native:true},0,()=>true);assert.equal(reads,0,'30Hz inter-frame gap must not start a full raster read');now=50;r.request({...binding,native:true},0,()=>true);await r.active;assert.equal(reads,1)}finally{await r.close()}
});

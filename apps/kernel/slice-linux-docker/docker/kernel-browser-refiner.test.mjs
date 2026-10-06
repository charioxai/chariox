// MD-DISPLAY-02/04: exact work cannot block input or commit an overtaken source.
import test from 'node:test';import assert from 'node:assert/strict';import {NativeRefiner} from './kernel-browser-refiner.mjs';
const binding={document:'d',policy:{},epoch:1,serial:1,scale:2};
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

// MD-DISPLAY-02/04: stalled encode does not own input/credit; loss resets deltas.
import test from 'node:test';import assert from 'node:assert/strict';import{MotionEncoder,CreditBudget}from'./kernel-browser-motion.mjs';
function fixture(){let offer,release;const calls=[];const encoder={async encode(image,b,key){calls.push({image,key});if(release)await new Promise(ok=>release=ok);return {key,data_base64:image}}};const source={subscribe(f){offer=f;return()=>offer=null},sample(){return null}};return {m:new MotionEncoder(source,encoder,{codec:'vp09.00.40.08',bitrate:4000000}),calls,offer:n=>offer({serial:n,data_base64:String(n)}),hold(){release=true},release(){const f=release;release=null;f()}};}
test('MD-DISPLAY superseded encoder work forces independent recovery without waiting input',async()=>{const f=fixture();f.hold();f.offer(1);f.offer(2);f.m.invalidate();f.offer(3);assert.equal(f.m.take(),null);f.release();await f.m.active;assert.deepEqual(f.calls.map(x=>x.key),[true,true]);assert.equal(f.m.take().serial,3);await f.m.close()});
test('MD-DISPLAY invalidation and close retire in-flight encoding',async()=>{const f=fixture();f.hold();f.offer(1);f.m.invalidate();const closing=f.m.close();f.release();await closing;assert.equal(f.m.frames.length,0)});
test('MD-DISPLAY exact repair retains a fully delivered video reference, avoiding a key per click',async()=>{const f=fixture();f.offer(1);await f.m.active;assert.equal(f.m.take().encoded.key,true);f.m.retireUnsent();f.offer(2);await f.m.active;assert.equal(f.m.take().encoded.key,false);await f.m.close()});
test('MD-DISPLAY full queue preserves decoder references and coalesces only unencoded source work',async()=>{const f=fixture();for(let n=1;n<=5;n++){f.offer(n);await f.m.active}assert.deepEqual(f.calls.map(x=>x.image),['1','2']);assert.equal(f.m.frames.length,2);assert.equal(f.m.take().serial,1);await f.m.active;assert.deepEqual(f.calls.map(x=>x.image),['1','2','5']);assert.equal(f.m.take().serial,2);assert.equal(f.m.take().serial,5);assert.equal(f.calls[2].key,false);await f.m.close()});

test('MD-DISPLAY credit pressure has hysteresis, bounded rate and recovery',()=>{let time=0;const r=new CreditBudget(2000000,()=>time);assert.equal(r.feedback(4),false);time=500;assert.equal(r.feedback(4),true);assert.equal(r.bitrate,1600000);for(let i=0;i<20;i++){time+=500;r.feedback(8)}assert.equal(r.bitrate,500000);r.feedback(0);time+=2000;r.feedback(0);assert.equal(r.bitrate,550000);assert.equal(r.feedback(NaN),false);for(let i=0;i<30;i++){time+=2000;r.feedback(0)}assert.equal(r.bitrate,2000000)});
test('MD-DISPLAY exact refinement retires stale work without re-encoding the same source serial',async()=>{const f=fixture();f.offer(1);await f.m.active;f.m.retireUnsent();f.offer(1);await f.m.active;assert.equal(f.calls.length,1);assert.equal(f.m.take(),null);f.offer(2);await f.m.active;assert.equal(f.m.take().encoded.key,true);await f.m.close()});

test('MD-DISPLAY stale encoded queue recovers latest source with a key and bounds memory',async()=>{
 let now=0,offer;const calls=[];let latest;
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={encode:async(image,bitrate,key)=>{calls.push({image,key});return{key,data_base64:image}}};
 const m=new MotionEncoder(source,encoder,{bitrate:2000000,codec:'avc1.420033',now:()=>now});
 try{
  latest={serial:1,data_base64:'one'};offer(latest);await m.active;
  now=350;latest={serial:2,data_base64:'two'};offer(latest);await m.active;
  assert.equal(m.take(),null);await m.active;
  assert.equal(m.take().serial,2);assert.equal(calls.at(-1).key,true);assert.equal(m.frames.length,0);
 }finally{await m.close()}
});

test('MD-DISPLAY delayed credits deliver a bounded recovery key instead of repeatedly starving continuous input',async()=>{
 let now=0,offer,latest;const calls=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={encode:async(image,bitrate,key)=>{calls.push({image,key});return{key,data_base64:image}}};
 const m=new MotionEncoder(source,encoder,{bitrate:32000000,codec:'avc1.420033',now:()=>now});
 try{
  latest={serial:1,data_base64:'one'};offer(latest);await m.active;
  now=150;latest={serial:2,data_base64:'two'};offer(latest);await m.active;
  assert.equal(m.take()?.serial,1,'admitted input delay must not discard every decoder recovery key');
  now=260;latest={serial:3,data_base64:'three'};offer(latest);await m.active;
  assert.equal(m.take(),null,'a stale dependent reference chain still retires');await m.active;
  now=410;latest={serial:4,data_base64:'four'};offer(latest);await m.active;
  assert.equal(m.take()?.serial,3,'recovery key must make bounded progress under sustained input');
  assert.equal(calls.filter(c=>c.image==='three').at(-1).key,true);
  assert.ok(m.frames.length<=2);
 }finally{await m.close()}
});

test('MD-DISPLAY eligible exact damage suppresses speculative encode and lost base reoffers an independent frame',async()=>{
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 const raw={format:'bgr0',width:1280,height:800,pixels:Buffer.alloc(1280*800*4),damage:[0,0,16,16]};
 const latest={raw,serial:2,width:1280,height:800,motion:true,data_base64:'native-source'};
 const calls=[];
 const encoder={encode:async(image,b,key)=>{calls.push(key);return{key,data_base64:'encoded'}},close:async()=>{}};
 const stream=new DisplayStream({codec:'avc1.420033',dependencies:true,device_scale_factor:1,bitrate:2000000},{encoder});
 stream.previous={signature:'viewer-base'};stream.compositorSerial=1;
 const source={subscribe:()=>()=>{},sample:()=>latest};
 const producer=new MotionEncoder(source,encoder,{codec:stream.codec,bitrate:stream.bitrate,shouldEncode:sample=>!stream.canPatchNative(sample)});
 stream.producer=producer;
 try{
  await producer.active;assert.deepEqual(calls,[],'a safely patchable update must not copy/encode a full motion raster');
  assert.equal(producer.take(),null);
  stream.invalidate();await producer.active;
  assert.deepEqual(calls,[true]);assert.equal(producer.take().encoded.key,true,'losing the base must reoffer skipped pixels independently');
 }finally{await stream.close()}
});

test('MP-10 lost queued stripe resets only affected row references',async()=>{
 let now=0,latest,offer;const resets=[];
 const source={subscribe(f){offer=f;return()=>{}},sample:()=>latest};
 const encoder={async encodeStripes(raw,bitrate,reset){resets.push(reset);return{stripes:(reset===true?[0,1,2,3,4,5,6,7]:reset.length?reset:[raw.row]).map(row=>({row,key:reset===true||reset.includes(row),data_base64:'AA=='}))}}};
 const m=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000,now:()=>now});
 try{
  latest={serial:1,raw:{row:0}};offer(latest);await m.active;assert.equal(m.take().encoded.stripes.length,8);
  latest={serial:2,raw:{row:0}};offer(latest);await m.active;
  latest={serial:3,raw:{row:0}};offer(latest);await m.active;
  now=150;assert.equal(m.take(),null);await m.active;
  assert.deepEqual(resets.at(-1),[0]);assert.deepEqual(m.take().encoded.stripes.map(r=>r.row),[0]);
  m.invalidate();await m.active;assert.equal(resets.at(-1),true,'protection/navigation fences reset every row');
 }finally{await m.close()}
});

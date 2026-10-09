// MD-DISPLAY-02/04: stalled encode does not own input/credit; loss resets deltas.
import test from 'node:test';import assert from 'node:assert/strict';import{MotionEncoder,CreditBudget}from'./kernel-browser-motion.mjs';
test('MP-08/MP-10/MP-11 dense native motion reuses one full-height codec; switching to sparse rows forces independent recovery',async()=>{
 let latest,offer;const calls=[];const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={async encode(raw,b,key){calls.push(['whole',key]);return{key,data_base64:'AA=='}},async encodeStripes(raw,b,reset){calls.push(['rows',reset]);return{stripes:[{row:0,key:reset===true,data_base64:'AA=='}]}}};
 const m=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000});
 try{
  for(const [serial,damage] of [[1,[0,0,1280,800]],[2,[0,0,1280,800]],[3,[0,0,32,32]]]){
   latest={serial,raw:{width:1280,height:800,damage,nativeEncode(){}}};offer(latest);await m.active;m.take();
  }
  assert.deepEqual(calls,[['whole',true],['whole',false],['rows',true]]);
 }finally{await m.close()}
});
test('MP-11 a rejected lossy mask emits only protected exact pixels and recovers with a key',async()=>{
 const {decodePng}=await import('./kernel-browser-pixels.mjs');
 let latest,offer;const calls=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={async encodeStripes(raw,b,reset){calls.push(reset);return calls.length===1?{dropped:true}:{stripes:[{row:0,key:reset===true,data_base64:'AA=='}]}}};
 const m=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000});
 try{
  latest={serial:1,raw:{width:128,height:128,pixels:Buffer.alloc(128*128*4)}};offer(latest);await m.active;
  const exact=m.take();assert.equal(exact.force_lossless,true);assert.equal(exact.encoded,undefined);assert.equal(exact.raw,undefined);
  assert.equal(decodePng(exact.data_base64).pixels[0],0);
  latest={...latest,serial:2};offer(latest);await m.active;assert.equal(m.take().encoded.stripes[0].key,true);
 }finally{await m.close()}
});
test('MP-11 oversized exact protection fallback fails closed without retaining a frame',async()=>{
 const latest={serial:1,data_base64:'A'.repeat(4*1024*1024+1)},source={subscribe:()=>()=>{},sample:()=>latest};
 const m=new MotionEncoder(source,{encode:async()=>({dropped:true})},{codec:'avc1.420033',bitrate:8000000});
 try{await m.active;assert.equal(m.frames.length,0);assert.throws(()=>m.take(),/motion encoder failed/)}finally{await m.close()}
});
test('MP-08/MP-10 raw-less stripe fallback recovers an oversized unsent full-video packet',async()=>{
 const latest={serial:1,data_base64:'source'},keys=[];
 const source={subscribe:()=>()=>{},sample:()=>latest};
 const encoder={async encode(image,bitrate,key){keys.push(key);return{key,data_base64:keys.length===1?'A'.repeat(1024*1024):'AA=='}}};
 const m=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000});
 try{await m.active;assert.deepEqual(keys,[true,true]);assert.equal(m.take().encoded.key,true)}finally{await m.close()}
});
for(const mode of ['queued','in-flight'])test('MP-08/MP-10 raw-less stripe negotiation recovers '+mode+' full-video work with a key',async()=>{
 let now=0,latest,offer,release;const keys=[];
 const source={subscribe(f){offer=f;return()=>{}},sample:()=>latest};
 const encoder={async encode(image,bitrate,key){keys.push(key);if(release)await new Promise(ok=>release=ok);return{key,data_base64:image}}};
 const m=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000,now:()=>now});
 try{
  latest={serial:1,data_base64:'base'};offer(latest);await m.active;
  if(mode!=='stale')assert.equal(m.take().encoded.key,true);
  if(mode==='in-flight')release=true;
  latest={serial:2,data_base64:'discarded'};offer(latest);
  if(mode==='in-flight'){m.retireUnsent();const done=release;release=null;done();await m.active}
  else {await m.active;m.retireUnsent()}
  latest={serial:3,data_base64:'recovery'};offer(latest);await m.active;
  assert.equal(m.take().encoded.key,true,'discarded full-video dependencies require a full-video key');
 }finally{await m.close()}
});
function fixture(){let offer,release;const calls=[];const encoder={async encode(image,b,key){calls.push({image,key});if(release)await new Promise(ok=>release=ok);return {key,data_base64:image}}};const source={subscribe(f){offer=f;return()=>offer=null},sample(){return null}};return {m:new MotionEncoder(source,encoder,{codec:'vp09.00.40.08',bitrate:4000000}),calls,offer:n=>offer({serial:n,data_base64:String(n)}),hold(){release=true},release(){const f=release;release=null;f()}};}
test('MD-DISPLAY superseded encoder work forces independent recovery without waiting input',async()=>{const f=fixture();f.hold();f.offer(1);f.offer(2);f.m.invalidate();f.offer(3);assert.equal(f.m.take(),null);f.release();await f.m.active;assert.deepEqual(f.calls.map(x=>x.key),[true,true]);assert.equal(f.m.take().serial,3);await f.m.close()});
test('MD-DISPLAY invalidation and close retire in-flight encoding',async()=>{const f=fixture();f.hold();f.offer(1);f.m.invalidate();const closing=f.m.close();f.release();await closing;assert.equal(f.m.frames.length,0)});
test('MD-DISPLAY exact repair retains a fully delivered video reference, avoiding a key per click',async()=>{const f=fixture();f.offer(1);await f.m.active;assert.equal(f.m.take().encoded.key,true);f.m.retireUnsent();f.offer(2);await f.m.active;assert.equal(f.m.take().encoded.key,false);await f.m.close()});
test('MD-DISPLAY full queue preserves decoder references and coalesces only unencoded source work',async()=>{const f=fixture();for(let n=1;n<=5;n++){f.offer(n);await f.m.active}assert.deepEqual(f.calls.map(x=>x.image),['1','2']);assert.equal(f.m.frames.length,2);assert.equal(f.m.take().serial,1);await f.m.active;assert.deepEqual(f.calls.map(x=>x.image),['1','2','5']);assert.equal(f.m.take().serial,2);assert.equal(f.m.take().serial,5);assert.equal(f.calls[2].key,false);await f.m.close()});

test('MD-DISPLAY credit pressure has hysteresis, bounded rate and recovery',()=>{let time=0;const r=new CreditBudget(2000000,()=>time);assert.equal(r.feedback(6),false);time=500;assert.equal(r.feedback(6),true);assert.equal(r.bitrate,1600000);for(let i=0;i<20;i++){time+=500;r.feedback(8)}assert.equal(r.bitrate,500000);r.feedback(0);time+=2000;r.feedback(0);assert.equal(r.bitrate,500000,'MP-10: recovery waits for 5 s without pressure');time+=3000;r.feedback(0);assert.equal(r.bitrate,550000);assert.equal(r.feedback(NaN),false);for(let i=0;i<30;i++){time+=5000;r.feedback(0)}assert.equal(r.bitrate,2000000)});

test('MP-08/MP-10 ordinary four-credit pipelining does not ratchet encoding to its bitrate floor',()=>{
 let now=0;const rate=new CreditBudget(8000000,()=>now);
 for(;now<10000;now+=16)rate.feedback(3);
 assert.equal(rate.bitrate,8000000,'three unacknowledged frames are a normal admitted window');
});
test('MD-DISPLAY exact refinement retires stale work without re-encoding the same source serial',async()=>{const f=fixture();f.offer(1);await f.m.active;f.m.retireUnsent();f.offer(1);await f.m.active;assert.equal(f.calls.length,1);assert.equal(f.m.take(),null);f.offer(2);await f.m.active;assert.equal(f.m.take().encoded.key,true);await f.m.close()});

// MP-08/MP-10: queued deltas of a live chain are delivered, never dropped
// for age; dropping forced an IDR and a rate cut on every credit delay.
test('MP-08/MP-10 aged queued deltas are delivered in order without an IDR or rate cut',async()=>{
 let now=0,offer,latest;const calls=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={encode:async(image,bitrate,key)=>{calls.push({image,key,bitrate});return{key,data_base64:image}}};
 const m=new MotionEncoder(source,encoder,{bitrate:8000000,codec:'avc1.420033',now:()=>now});
 try{
  latest={serial:1,data_base64:'one'};offer(latest);await m.active;
  now=350;latest={serial:2,data_base64:'two'};offer(latest);await m.active;
  now=900;latest={serial:3,data_base64:'three'};offer(latest);await m.active;
  assert.equal(m.take().serial,1);assert.equal(m.take().serial,2);await m.active;assert.equal(m.take().serial,3);
  assert.deepEqual(calls.map(c=>c.key),[true,false,false]);assert.ok(calls.every(c=>c.bitrate===8000000));assert.ok(m.frames.length<=2);
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
  m.retireUnsent();latest={serial:4,raw:{row:0}};offer(latest);await m.active;
  assert.deepEqual(resets.at(-1),[0]);assert.deepEqual(m.take().encoded.stripes.map(r=>r.row),[0]);
  m.invalidate();await m.active;assert.equal(resets.at(-1),true,'protection/navigation fences reset every row');
 }finally{await m.close()}
});

// MP-08/MP-10/MP-11: review15:28, disjoint queued/incoming dependency retirement.
for(const native of [false,true])test('MP-11 overflow recovers incoming and queued stripe references '+native,async()=>{
 const {MotionEncoder:Encoder}=await import(process.env.MP_MOTION_MODULE??'./kernel-browser-motion.mjs');
 let latest,offer;const resets=[],discarded=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={discard:x=>discarded.push(x),async encodeStripes(raw,bitrate,reset){
  resets.push(reset);const rows=reset===true?[0,1,2,3,4,5,6,7]:reset.length?reset:[raw.row];
  const recovery=reset===true||reset.length>0;
  return {stripes:rows.map(row=>({row,key:recovery,...(!native?{data_base64:recovery?'AA==':'A'.repeat(600000)}:{})})),...(native?{packet:{name:String(raw.serial),length:recovery?128:600000}}:{})};
 }};
 const m=new Encoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000});
 try{
  latest={serial:1,raw:{row:0,serial:1}};offer(latest);await m.active;m.take();
  latest={serial:2,raw:{row:0,serial:2}};offer(latest);await m.active;
  latest={serial:3,raw:{row:1,serial:3}};offer(latest);await m.active;
  const recovered=m.take();assert.ok(recovered.encoded.stripes.some(r=>r.row===0&&r.key),'queued row0 must recover');
  assert.ok(recovered.encoded.stripes.some(r=>r.row===1&&r.key),'discarded incoming row1 must recover too');
  latest={serial:4,raw:{row:1,serial:4}};offer(latest);await m.active;assert.equal(m.take().encoded.stripes[0].key,false,'next row1 packet may follow the delivered recovery');
 }finally{await m.close()}
});

test('MP-11 dropped private codec output retains its lease through exact fallback materialization',async()=>{
 let refs=1,release,offer;
 const raw={width:16,height:16,retain(){refs++},release(){assert(refs>0);refs--}};
 Object.defineProperty(raw,'pixels',{get(){assert(refs>0,'fallback read needs a live immutable lease');return Buffer.alloc(16*16*4)}});
 const source={sample:()=>null,subscribe:fn=>{offer=fn;return()=>{}}};
 const m=new MotionEncoder(source,{encode:()=>new Promise(done=>release=done)},{bitrate:8000000,codec:'avc1.420033'});
 try{offer({serial:1,raw});raw.release();assert.equal(refs,1);release({dropped:true});await m.active;assert.equal(m.failure,undefined);assert.equal(m.take().force_lossless,true);assert.equal(refs,0)}finally{await m.close()}
});

test('MP-08/MP-10/MP-11 motion readiness parks a credit until encoded pixels, patchable source, cancellation or close',async()=>{
 const f=fixture();
 try{
  let done=false;const ready=f.m.waitReady(1000).then(()=>done=true);await new Promise(r=>setImmediate(r));assert.equal(done,false);
  f.offer(1);await f.m.active;await ready;assert.equal(done,true);f.m.take();
  const abort=new AbortController();const cancelled=f.m.waitReady(1000,abort.signal);abort.abort();await cancelled;assert.equal(f.m.readyWaiters.size,0);
  f.m.shouldEncode=()=>false;const patch=f.m.waitReady(1000);f.offer(2);await patch;
  const closing=f.m.waitReady(1000);await f.m.close();await closing;assert.equal(f.m.readyWaiters.size,0);
 }finally{await f.m.close()}
});

test('MP-08 limited vertical native motion keeps row reuse; malformed hints take conservative whole video',async()=>{
 let latest,offer;const calls=[],source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={async encode(raw,b,key){calls.push(['whole',key]);return{key,data_base64:'AA=='}},async encodeStripes(raw,b,key){calls.push(['rows',key]);return{stripes:[{row:0,key:key===true,data_base64:'AA=='}]}}};
 const motion=new MotionEncoder(source,encoder,{stripes:true,codec:'avc1.420033',bitrate:8000000});
 try{
  let serial=0;for(const height of [420,800,-1,801,undefined]){latest={serial:++serial,raw:{width:1280,height:800,damage:[0,0,1280,800],motion_height:height,nativeEncode(){}}};offer(latest);await motion.active;motion.take();}
  assert.deepEqual(calls,[['rows',true],['whole',true],['whole',false],['whole',false],['whole',false]]);
 }finally{await motion.close();}
});

test('MP-08/MP-10/MP-11 rejected native codec carries indexed exact PNG through display credit',async()=>{
 const {deflateSync,crc32}=await import('node:zlib');
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 const {decodePng}=await import('./kernel-browser-pixels.mjs');
 const chunk=(type,data)=>{const body=Buffer.concat([Buffer.from(type),data]),out=Buffer.alloc(body.length+8);out.writeUInt32BE(data.length);body.copy(out,4);out.writeUInt32BE(crc32(body),out.length-4);return out};
 const header=Buffer.alloc(13);header.writeUInt32BE(128);header.writeUInt32BE(128,4);header[8]=8;header[9]=3;
 const indexed=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('PLTE',Buffer.from([0,0,0])),chunk('IDAT',deflateSync(Buffer.alloc(129*128))),chunk('IEND',Buffer.alloc(0))]).toString('base64');
 // An actual indexed native PNG; the fallback decoder expands it exactly.
 const decoded=decodePng(indexed);assert.equal(decoded.width,128);assert.ok(decoded.pixels.every((v,i)=>i%4===3?v===255:v===0));
 let offer;const source={subscribe:f=>(offer=f,()=>{}),sample:()=>null};
 const encoder={nativeSession:'native',encode:async()=>({dropped:true}),close:async()=>{}};
 const motion=new MotionEncoder(source,encoder,{codec:'avc1.420033',bitrate:8000000});
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033',dependencies:true},{encoder,now:()=>0,wait:async()=>{}});
 let exactCalls=0;
 try{
  offer({serial:1,data_base64:'signature',raw:{width:128,height:128,nativeEncode(){},nativeExact:async q=>{exactCalls++;assert.equal(q.patch,false);return {width:128,height:128,native_exact:true,data_base64:indexed,repair_tiles:[]}}}});
  await motion.active;const exact=motion.take();
  const frame=await stream.frame({...exact,generation:1},'d',0);
  assert.equal(exactCalls,1);assert.equal(frame.kind,'png');assert.equal(frame.data_base64,indexed);assert.equal(stream.exact,true);assert.equal(exact.raw,undefined);
 }finally{await motion.close();await stream.close()}
});
test('MP-11 an exact encode overlapping the producer skips one sample instead of failing motion',async()=>{
 const {BrowserEncoder}=await import('./kernel-browser-webcodecs.mjs');
 let latest,offer,release;const held=new Promise(r=>release=r),calls=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder=new BrowserEncoder({},'target',{async encode(image,b,key){calls.push([image,key]);if(image==='exact')await held;return {key,data_base64:'AA=='}},close:async()=>{}});
 encoder.fallbackOnly=true;
 const m=new MotionEncoder(source,encoder,{codec:'avc1.420033',bitrate:8000000});
 try{
  const exact=encoder.encode('exact',8000000,true,'avc1.420033');
  latest={serial:1,data_base64:'1'};offer(latest);await m.active;
  assert.equal(m.take(),null);assert.equal(m.failure,undefined,'busy is transient');
  release();await exact;
  latest={serial:2,data_base64:'2'};offer(latest);await m.active;
  assert.equal(m.take().encoded.key,true);assert.deepEqual(calls.map(c=>c[0]),['exact','2']);
 }finally{await m.close();await encoder.close()}
});
// MP-08/MP-10: native x264 retunes rate live; only other encoders re-key.
for(const native of [true,false])test('MP-08/MP-10 congestion rate change keeps the native reference chain '+native,async()=>{
 let offer,latest,now=0;const calls=[];
 const source={subscribe:f=>{offer=f;return()=>{}},sample:()=>latest};
 const encoder={encode:async(image,bitrate,key)=>{calls.push({key,bitrate});return{key,data_base64:'x'}}};
 const m=new MotionEncoder(source,encoder,{bitrate:8000000,codec:'avc1.420033',now:()=>now});
 const sample=serial=>native?{serial,raw:{nativeEncode(){},width:1,height:1}}:{serial,data_base64:String(serial)};
 try{
  latest=sample(1);offer(latest);await m.active;m.take();
  m.feedback(8);now=600;m.feedback(8);
  latest=sample(2);offer(latest);await m.active;
  assert.ok(calls[1].bitrate<8000000,'congestion lowers the rate');assert.equal(calls[1].key,!native);
 }finally{await m.close()}
});

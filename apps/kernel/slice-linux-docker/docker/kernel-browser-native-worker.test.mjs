// MP-08/MP-10/MP-11: native bridge retains authority/leases without pixels.
import test from 'node:test';import assert from 'node:assert/strict';
import {NativeWorkerControl,nativeExactReplyLimit} from './kernel-browser-native-worker.mjs';
import {NativePipe} from './kernel-browser-native-pipe.mjs';
import {PortableEncoder,DisplayStream} from './kernel-browser-display.mjs';
import {NativeRefiner} from './kernel-browser-refiner.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
test('MP-08/MP-10/MP-11 Retina exact replies fit private IPC without enlarging raw or encode bounds',async()=>{
 const writes=[],bridge=new NativeWorkerControl({stdin:{write(line){writes.push(JSON.parse(line))}}},()=>{});
 const exact=bridge.request('exact',{serial:1});
 // A full 2560x1600 random RGB repair carries ~16.4 MB of base64 WebP,
 // above both the old 8 MiB reply cap and the raw raster framing cap.
 const value={native_exact:true,width:2560,height:1600,native_repair:true,repair_tiles:[{data_base64:'A'.repeat(2560*1600*4)}]};
 const bytes=Buffer.from(JSON.stringify(value)),id=writes[0].exact.id;
 const header={reply:id,length:bytes.length},text=Buffer.from(JSON.stringify(header)),length=Buffer.alloc(4);length.writeUInt32BE(text.length);
 const pipe=new NativePipe(h=>bridge.validate(h),(h,b)=>bridge.receive(h,b));
 try {
  for(const size of [0,NaN,1.5,nativeExactReplyLimit+1])assert.throws(()=>bridge.validate({reply:id,length:size}),/binding/);
  assert.throws(()=>bridge.validate({reply:id+1,length:bytes.length}),/binding/);
  for(const part of [length,text,bytes])pipe.push(part);
  assert.deepEqual(await exact,value);
  const encoded=bridge.request('encode',{serial:1});
  assert.throws(()=>bridge.validate({reply:writes[1].encode.id,length:8*1024*1024+1}),/binding/);
  bridge.close();await assert.rejects(encoded,/closed/);
  const rawHeader=Buffer.from(JSON.stringify({length:2560*1600*4+1})),rawLength=Buffer.alloc(4);rawLength.writeUInt32BE(rawHeader.length);
  assert.throws(()=>new NativePipe(()=>{},()=>{}).push(Buffer.concat([rawLength,rawHeader])),/frame/);
 }finally{bridge.close();await exact.catch(()=>{})}
});
test('MP-11 native control binds every reply and settles pending work on death',async()=>{
 const writes=[],child={stdin:{write(line){writes.push(JSON.parse(line));}}};
 const bridge=new NativeWorkerControl(child,()=>{});
 const pending=bridge.request('encode',{serial:1});
 const id=writes[0].encode.id;
 assert.throws(()=>bridge.validate({reply:id+1,length:1}),/binding/);
 bridge.validate({reply:id,length:2});bridge.receive({reply:id},Buffer.from('{}'));assert.deepEqual(await pending,{});
 const dying=bridge.request('exact',{serial:1});bridge.close();await assert.rejects(dying,/closed/);assert.equal(bridge.pending.size,0);
});
test('MP-08/MP-10/MP-11 native motion never starts Python or reads the raster in Node',async()=>{
 const encoder=new PortableEncoder();let called;
 const raw={nativeEncode:async request=>{called=request;return {backend:'native-x264',converter:'libyuv',workers:1,stripes:[],revision:3}},[displayMaskRegions]:[{x:9,y:3,width:1,height:1}]};
 Object.defineProperty(raw,'pixels',{get(){throw Error('MP-11: Node raster read')}});
 try{assert.deepEqual(await encoder.encodeStripes(raw,8000000,[2]),{stripes:[],native_revision:3,native_deliver:undefined});assert.equal(encoder.child,null);assert.deepEqual(called.regions,raw[displayMaskRegions]);assert.deepEqual(called.reset,[2]);assert.equal(called.encoder,encoder.nativeSession);}finally{await encoder.close()}
});
test('MP-08/MP-10 the native worker reports its runtime-loaded encoder (OpenH264 default, x264 optional)',async()=>{
 for(const [backend,label] of [['native-openh264','openh264'],['native-x264','x264'],['native-vaapi','vaapi']]){
  const encoder=new PortableEncoder();const raw={nativeEncode:async()=>({backend,converter:'libyuv',workers:1,stripes:[],revision:1}),[displayMaskRegions]:[]};
  try{await encoder.encodeStripes(raw,8000000,[]);assert.equal(encoder.backend,label);}finally{await encoder.close()}
 }
 const encoder=new PortableEncoder();const raw={nativeEncode:async()=>({backend:'native-other',stripes:[],revision:1}),[displayMaskRegions]:[]};
 try{await assert.rejects(encoder.encodeStripes(raw,8000000,[]),/MP-11: native codec reply/);}finally{await encoder.close()}
});
test('MP-08/MP-10 native exact preparation bypasses Node pixel workers and retains its immutable lease',async()=>{
 let held=0,prepared;const raw={width:1280,height:800,retain(){held++},release(){held--},nativeExact:async request=>{prepared=request;return {native_exact:true,data_base64:'exact',width:1280,height:800}},[displayMaskRegions]:[]};
 Object.defineProperty(raw,'pixels',{get(){throw Error('MP-10: Node pixels')}});
 const sample={raw,signature:'serial-1',motion:true};
 const refiner=new NativeRefiner(async()=>sample,{now:()=>100,pixels:{run(){throw Error('MP-10: Node render')},close:async()=>{}},prepareTiles:true});
 try{refiner.request({native:true,sample,source:{},document:'d',policy:{},epoch:0,serial:1,encoder:'e',repairLimit:192000},0,()=>true);await refiner.active;assert.equal(refiner.failure,undefined);assert.equal(held,0);assert.equal(refiner.latest.native_exact,true);assert.equal(refiner.latest.data_base64,'exact');assert.equal(prepared.encoder,'e');assert.equal(prepared.repair_only,true,'refinement prepares only the viewport tiles; opaque fallback keeps its full PNG path');}finally{await refiner.close()}
});
test('MP-08/MP-10 native exact frames retain ordinary sequencing and byte pacing without Node decode',async()=>{
 const encoder={close:async()=>{}};const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,codec:'avc1.420033',bitrate:8000000},{encoder,now:()=>0,wait:async()=>{}});
 stream.document_id='d';stream.sequence=1;stream.previous={signature:'lossy'};
 const source={native_exact:true,settled_verified:true,width:1280,height:800,data_base64:'AA==',generation:1,refinement_serial:4};
 try{const frame=await stream.frame(source,'d',1);assert.equal(frame.kind,'png');assert.equal(frame.sequence,2);assert.equal(stream.exact,true);assert.equal(stream.pixels.worker,null);}finally{await stream.close()}
});

test('MP-11 native encoder close retires only its private session',async()=>{
 const encoder=new PortableEncoder();const retired=[];const raw={nativeRetire:name=>retired.push(name),nativeEncode:async()=>({backend:'native-x264',stripes:[],revision:1})};
 await encoder.encodeStripes(raw,8000000,false);await encoder.close();await encoder.close();assert.deepEqual(retired,[encoder.nativeSession]);
});


test('MP-08/MP-10/MP-11 native control flushes ordered frame notifications with the next request',async()=>{
 const writes=[],child={stdin:{write(bytes){writes.push(bytes)}}};
 const bridge=new NativeWorkerControl(child,()=>{});
 bridge.commit('e',7,false);bridge.delivered('e',4);bridge.retire('old');
 const pending=bridge.request('encode',{serial:8,encoder:'e'});
 try{
  assert.equal(writes.length,1,'frame notifications and request share one pipe write');
  assert.deepEqual(writes[0].trim().split('\n').map(line=>JSON.parse(line)),[
   {commit:'e',serial:7,admit:false},{delivered:'e',revision:4},{retire:'old'},{encode:{id:1,serial:8,encoder:'e'}}]);
  bridge.receive({reply:1},Buffer.from('{}'));await pending;
 }finally{bridge.close();await pending.catch(()=>{})}
});
test('MP-11 batches are bounded, lone notifications flush, urgent wheel keeps order, close retires pending flush',async()=>{
 const writes=[],bridge=new NativeWorkerControl({stdin:{write:s=>writes.push(s)}},()=>{});
 for(let i=0;i<64;i++)bridge.delivered('e',i);
 assert.equal(writes.length,1);assert.equal(writes[0].trim().split('\n').length,64);
 bridge.commit('e',65);await new Promise(r=>setImmediate(r));assert.equal(writes.length,2);
 bridge.delivered('e',66);bridge.notify({wheel:[1,2,0,1]},true);assert.deepEqual(writes[2].trim().split('\n').map(s=>JSON.parse(s)),[{delivered:'e',revision:66},{wheel:[1,2,0,1]}]);
 bridge.delivered('e',67);bridge.close();await new Promise(r=>setImmediate(r));assert.equal(writes.length,3);assert.equal(bridge.notifications.length,0);
});
test('MP-08/MP-10: Retina whole-frame motion encodes at the reduced geometry; stripes and masks stay native',async()=>{
 const encoder=new PortableEncoder(),requests=[];
 const raw=(width,regions=[])=>({width,height:width===2560?1600:800,[displayMaskRegions]:regions,nativeEncode:async request=>{requests.push(request);return {backend:'native-x264',stripes:[],revision:1}}});
 await encoder.nativeEncode(raw(2560),8000000,false,false);
 await encoder.nativeEncode(raw(2560),8000000,false,true);
 await encoder.nativeEncode(raw(2560,[{x:0,y:0,width:10,height:10}]),8000000,false,false);
 await encoder.nativeEncode(raw(1280),8000000,false,false);
 assert.deepEqual(requests.map(r=>r.reduced===true),[true,false,false,false]);
});

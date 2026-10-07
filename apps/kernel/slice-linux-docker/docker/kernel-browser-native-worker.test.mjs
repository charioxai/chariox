// MP-08/MP-10/MP-11: native bridge retains authority/leases without pixels.
import test from 'node:test';import assert from 'node:assert/strict';
import {NativeWorkerControl} from './kernel-browser-native-worker.mjs';
import {PortableEncoder,DisplayStream} from './kernel-browser-display.mjs';
import {NativeRefiner} from './kernel-browser-refiner.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
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
test('MP-08/MP-10 native exact preparation bypasses Node pixel workers and retains its immutable lease',async()=>{
 let held=0,prepared;const raw={width:1280,height:800,retain(){held++},release(){held--},nativeExact:async request=>{prepared=request;return {native_exact:true,data_base64:'exact',width:1280,height:800}},[displayMaskRegions]:[]};
 Object.defineProperty(raw,'pixels',{get(){throw Error('MP-10: Node pixels')}});
 const sample={raw,signature:'serial-1',motion:true};
 const refiner=new NativeRefiner(async()=>sample,{now:()=>100,pixels:{run(){throw Error('MP-10: Node render')},close:async()=>{}},prepareTiles:true});
 try{refiner.request({native:true,sample,source:{},document:'d',policy:{},epoch:0,serial:1,encoder:'e',repairLimit:192000},0,()=>true);await refiner.active;assert.equal(refiner.failure,undefined);assert.equal(held,0);assert.equal(refiner.latest.native_exact,true);assert.equal(refiner.latest.data_base64,'exact');assert.equal(prepared.encoder,'e');}finally{await refiner.close()}
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

// MP-08/MP-11: exact native/CDP fill fences, supplementary to real live drills.
import test from 'node:test';import assert from 'node:assert/strict';
import {captureProtectionFence,NativeRegionProtection} from './kernel-browser-region-protection.mjs';
const policy={unknown:false,targets:[],values:[]};
const page=regions=>({document_id:'D',viewport:[1280,800],regions,withheld:[]});
test('MP-08/MP-11 native fill fence masks only field pixels at DPR2',async()=>{
 const fence=await captureProtectionFence(null,'S','T',policy,{measure:async()=>page([[10,20,100,30]])});
 assert.deepEqual(await fence.afterCapture({width:2560,height:1600}),[{x:20,y:40,width:200,height:60}]);
 const source=new NativeRegionProtection(null,'S',{targetId:'T',policy,measure:async()=>page([[10,20,100,30]])});
 await source.refresh();assert.deepEqual(await source.regions({width:1280,height:800,captured_ms:source.beforeAt+1}),[{x:10,y:20,width:100,height:30}]);
});
test('MP-08/MP-11 password-to-plain and moved fields retry without a page mask',async()=>{
 for(const after of [[[10,20,100,30]],[[11,20,100,30]]]){
  let n=0;const before=after[0][0]===10?[]:[[10,20,100,30]];
  const fence=await captureProtectionFence(null,'S','T',policy,{measure:async()=>page(n++?after:before)});
  await assert.rejects(fence.afterCapture({width:1280,height:800}),e=>e.code==='fill_capture_retry');
 }
});
test('MP-08/MP-11 unbound or stale native captures retry instead of broad masking',async()=>{
 await assert.rejects(captureProtectionFence(null,'S','T',{...policy,unknown:true}),e=>e.code==='fill_capture_retry');
 const source=new NativeRegionProtection(null,'S',{targetId:'T',policy,measure:async()=>page([])});await source.refresh();
 await assert.rejects(source.regions({width:1280,height:800,captured_ms:source.beforeAt-1}),e=>e.code==='fill_capture_retry');
 assert.deepEqual(await source.regions({width:1280,height:800,captured_ms:source.beforeAt+1}),[]);
});

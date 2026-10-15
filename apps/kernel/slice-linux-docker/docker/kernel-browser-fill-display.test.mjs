// MP-08/MP-10/MP-11: display consumes only the shared Vault fill-target boxes.
// Exact native/CDP fill fences are supplementary to real live drills.
import test from 'node:test';
import assert from 'node:assert/strict';
import {captureRegionMasks, captureProtectionFence, NativeRegionProtection} from './kernel-browser-region-protection.mjs';

test('MP-11 display has no independent generic field, frame or shadow masks', async () => {
  const calls=[];
  const connection={async send(method){calls.push(method);if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe'},{nodeId:3,localName:'input',attributes:['type','password']}]}};if(method==='DOM.querySelectorAll')return {nodeIds:[2,3]};return {model:{border:[0,0,100,0,100,100,0,100]}};}};
  const guard=await captureRegionMasks(connection,'s');
  assert.deepEqual(await guard.afterCapture({width:1280,height:800}),[]);
  assert.deepEqual(calls,[], 'the fill-target collector owns inspection');
});

test('MP-11 changing field boxes refuse a capture instead of masking the frame', async () => {
  let count=0;
  const measure=async()=>({viewport:[1280,800],regions:[[10+(count++?10:0),20,30,40]],withheld:[]});
  const fence=await captureProtectionFence({},'s','t',{targets:[{}]}, {measure});
  await assert.rejects(fence.afterCapture({width:1280,height:800}), /fill-target capture/);
});

test('MP-11 unavailable collector refuses capture without inventing a full-frame mask', async () => {
  await assert.rejects(async()=>{const fence=await captureProtectionFence({},'s','t',{targets:[{}]}, {measure:async()=>{throw Error('unavailable');}});await fence.afterCapture({width:1280,height:800});}, /fill-target capture/);
});

const policy={unknown:false,targets:[],values:[]};
const page=regions=>({document_id:'D',viewport:[1280,800],regions,withheld:[]});
test('MP-08/MP-11 native fill fence masks only field pixels at DPR2',async()=>{
 const fence=await captureProtectionFence(null,'S','T',policy,{measure:async()=>page([[10,20,100,30]])});
 assert.deepEqual(await fence.afterCapture({width:2560,height:1600}),[{x:20,y:40,width:200,height:60}]);
 const source=new NativeRegionProtection(null,'S',{policy});source.measure=async()=>[[10,20,100,30]];
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
 const source=new NativeRegionProtection(null,'S',{policy});source.measure=async()=>[];await source.refresh();
 await assert.rejects(source.regions({width:1280,height:800,captured_ms:source.beforeAt-1}),e=>e.code==='fill_capture_retry');
 assert.deepEqual(await source.regions({width:1280,height:800,captured_ms:source.beforeAt+1}),[]);
});

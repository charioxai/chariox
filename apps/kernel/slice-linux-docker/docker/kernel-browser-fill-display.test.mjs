// MP-08/MP-10/MP-11: display consumes only the shared Vault fill-target boxes.
import test from 'node:test';
import assert from 'node:assert/strict';
import {captureRegionMasks, captureProtectionFence} from './kernel-browser-region-protection.mjs';

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

// MP-10: codec noise and repeated pictures must not inflate content cadence.
import test from 'node:test';import assert from 'node:assert/strict';
import {changedPixels} from './motion-samples.mjs';
test('MP-10 content cadence excludes repeated pixels and bounded codec noise',()=>{
 const a=new Uint8ClampedArray(4000),noise=a.map((v,i)=>i%4===3?255:8),motion=noise.slice();
 assert.equal(changedPixels(null,a),true);assert.equal(changedPixels(a,a),false);
 assert.equal(changedPixels(a,noise),false);motion[0]=20;assert.equal(changedPixels(a,motion),false);
 motion[4]=20;assert.equal(changedPixels(a,motion),true);
});

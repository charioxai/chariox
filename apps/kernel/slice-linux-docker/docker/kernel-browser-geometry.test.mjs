// MP-08/MP-10: Browser density cannot change the shared physical page transform.
import test from 'node:test';
import assert from 'node:assert/strict';
import {displayDeviceMetrics} from './kernel-browser-geometry.mjs';
for(const dpr of [1,2])test(`MP-10 Browser DPR${dpr} preserves the native page transform`,()=>{
  assert.deepEqual(displayDeviceMetrics(1280,800,dpr),{width:1280,height:800,deviceScaleFactor:dpr,scale:1,mobile:false});
});

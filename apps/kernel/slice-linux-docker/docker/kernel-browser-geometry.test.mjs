// MP-08/MP-10: a Browser density must fit the shared physical desktop.
import test from 'node:test';
import assert from 'node:assert/strict';
import {displayDeviceMetrics} from './kernel-browser-geometry.mjs';
for(const hostScale of [1,2])for(const dpr of [1,2])test(`MP-10 shared desktop fits Browser DPR${dpr} at host scale${hostScale}`,()=>{
  const metrics=displayDeviceMetrics(1280,800,dpr,hostScale);
  assert.equal(metrics.deviceScaleFactor,dpr);
  assert.equal(metrics.scale*dpr,hostScale,'emulated density must not enlarge or crop the physical viewport');
});

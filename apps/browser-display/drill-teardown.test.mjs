// MP-08/MP-10/MP-11: supplementary teardown regression, never acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {collectRelayDiagnostics} from './drill-teardown.mjs';

test('MP-11 viewer telemetry records successful completion and cannot stop early-failure cleanup', async () => {
  for (const page of [undefined, {evaluate(){throw Error('launch failure')}}, {evaluate:async()=>{throw Error('viewer closed')}}]) {
    const receipt={binary_relay_frames:3};let cleaned=false;
    await collectRelayDiagnostics(page,receipt);
    cleaned=true;
    assert.equal(cleaned,true);assert.equal(receipt.binary_relay_frames,3);
  }
  const receipt={};await collectRelayDiagnostics({evaluate:async()=>7},receipt);
  assert.equal(receipt.binary_relay_frames,7);
});

test('MP-11 invalid viewer counters preserve the last trustworthy sample', async () => {
  for (const count of [undefined, NaN, Infinity, -1, 1.5, '7']) {
    const receipt={binary_relay_frames:3};
    await collectRelayDiagnostics({evaluate:async()=>count},receipt);
    assert.equal(receipt.binary_relay_frames,3);
  }
});

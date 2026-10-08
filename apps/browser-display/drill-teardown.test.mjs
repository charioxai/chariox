// MP-08/MP-10/MP-11: supplementary teardown regression, never acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
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

test('MP-11 the real drill owns the viewer page outside try and guards its teardown telemetry', async () => {
  const source=await readFile(new URL('./drill.mjs',import.meta.url),'utf8');
  const resources=source.match(/^let kernel,[^;]+;/m)?.[0];
  assert.match(resources??'',/\bpage\b/);
  assert.doesNotMatch(source,/const page=await browser\.contexts\(\)\[0\]\.newPage/);
  assert.match(source,/finally \{\s*await collectRelayDiagnostics\(page,receipt\);/);
});

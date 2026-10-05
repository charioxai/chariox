import test from 'node:test';
import assert from 'node:assert/strict';
import { plannedNativeAdapter } from './adapter.mjs';
test('MD-DISPLAY native stubs cannot masquerade as available capture or encoders',async()=>{
 for(const os of ['darwin','win32']){const adapter=plannedNativeAdapter(os);assert.equal(adapter.available,false);await assert.rejects(adapter.captureProtected(),/design stub/);await assert.rejects(adapter.encodeProtected(),/design stub/);await adapter.close()}
 assert.throws(()=>plannedNativeAdapter('unknown'));
});

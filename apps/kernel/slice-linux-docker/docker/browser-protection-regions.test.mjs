// MP-08/MP-11: unchanged-capture fence regression.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ProtectionGate,measureBrowserProtection,awaitPresented} from './browser-protection-regions.mjs';
test('stream gate adopts only stable protection and releases only verified frames', async () => {
  const sequence = ['A', 'A', 'A', 'B', 'B', null, 'B'];
  let clock = 0;
  const gate = new ProtectionGate(async () => { const next = sequence.shift(); if (next === null) throw new Error('unbound'); return { pages: [next] }; });
  const steps = [];
  for (let i = 0; i < 7; i++) steps.push({ ...(await gate.step(() => ++clock)), serial: gate.protectionSerial });
  assert.deepEqual(steps.map(({ verified, changed, serial }) => [verified, changed, serial]), [
    [null, false, 0], // first sight: withheld
    [null, true, 1],  // stable across a presented frame: adopt
    [1, false, 1],    // frames captured with 1 before this step are released
    [null, true, 0],  // changed layout: drop frames, withhold
    [null, true, 2],  // new layout stable: adopt
    [null, true, 0],  // unbound measurement: withhold
    [null, false, 0], // one sighting is not enough
  ]);
  assert.equal(gate.protection, null);
});

test('MP-08/MP-10/MP-11 no filled browser field needs no renderer geometry or presentation fence',async()=>{
 const browser={fillTargets:new Map(),ensureConnection:async()=>{assert.fail('no filled field requires CDP work')}};
 const policy={unknown:false,values:['public-registration-only'],targets:[{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc'}]};
 assert.deepEqual(await measureBrowserProtection(browser,policy),{pages:[]});
 await awaitPresented(browser,[]);
 browser.fillTargets.set('pending',{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc',value_hash:'public-hash',pending:true});
 await assert.rejects(measureBrowserProtection(browser,policy),/no filled field requires CDP work/,'pending fills retain the capture proof');
 await assert.rejects(measureBrowserProtection(browser,{...policy,unknown:true}),/fill policy unavailable/);
});

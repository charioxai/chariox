// MP-08/MP-11: unchanged-capture fence regression.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ProtectionGate, awaitPresented} from './browser-protection-regions.mjs';

for (const result of [true,false,'failure']) {
  test(`MP-08/MP-11: presentation session settles on ${result}`, async () => {
    const calls=[];
    const connection={async send(method,params,sessionId) {
      calls.push({method,params,sessionId});
      if(method==='Target.attachToTarget')return {sessionId:'owned-session'};
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      if(method==='Runtime.evaluate') {
        assert.equal(sessionId,'owned-session');assert.equal(params.awaitPromise,true);
        assert.match(params.expression,/setTimeout\(\(\) => resolve\(false\), 500\)/);
        if(result==='failure')throw Error('MP-11 synthetic CDP failure');
        return {result:{value:result}};
      }
      return {};
    }};
    const browser={async ensureConnection(){return connection}};
    if(result===true)await awaitPresented(browser,[{target_id:'visible-page'}]);
    else await assert.rejects(awaitPresented(browser,[{target_id:'visible-page'}]),result===false?/did not present/:/synthetic CDP failure/);
    assert.deepEqual(calls[0],{method:'Target.attachToTarget',params:{targetId:'visible-page',flatten:true},sessionId:undefined});
    assert.deepEqual(calls.at(-1),{method:'Target.detachFromTarget',params:{sessionId:'owned-session'},sessionId:undefined});
  });
}
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

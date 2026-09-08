import assert from 'node:assert/strict';
import test from 'node:test';
import {completeCookieImport} from './cookie-import-completion.mjs';

const receipt = 'a'.repeat(64);
for (const failedAt of [null,'outcome','discard','clear']) {
  test(`completion preserves durable ordering with failure at ${failedAt}`,async () => {
    const calls = [];
    const step = name => async () => {
      calls.push(name);
      if (name === failedAt) throw new Error('synthetic-private-detail');
    };
    const operation = completeCookieImport({receipt,
      journal:{discard:async actual => {assert.equal(actual,receipt); await step('discard')();}},
      recordOutcome:step('outcome'),clearPending:step('clear')});
    if (failedAt) await assert.rejects(operation,error => {
      assert.equal(error.message,'cookie_import_completion_failed');
      assert.equal(error.recoveryRequired,true);
      assert.equal(error.cause,undefined);
      return true;
    });
    else await operation;
    const order = ['outcome','discard','clear'];
    assert.deepEqual(calls,failedAt ? order.slice(0,order.indexOf(failedAt)+1) : order);
  });
}

test('invalid completion never records an outcome',async () => {
  let called = false;
  await assert.rejects(completeCookieImport({receipt:'invalid',journal:{discard(){}},
    recordOutcome:async()=>{called=true;},clearPending:async()=>{called=true;}}),
  {code:'cookie_import_completion_failed'});
  assert.equal(called,false);
});

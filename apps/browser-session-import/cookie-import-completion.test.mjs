import assert from 'node:assert/strict';
import test from 'node:test';
import {completeCookieImport,resumeCookieImportCleanup} from './cookie-import-completion.mjs';

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

test('restart after journal deletion clears only with a durable matching outcome',async () => {
  for (const confirmed of [false,true]) {
    let cleared = false;
    const operation = resumeCookieImportCleanup({
      journal:{read:async()=>null,discard:async()=>assert.fail('already removed')},
      confirmOutcome:async()=>confirmed,clearPending:async()=>{cleared=true;}});
    if (confirmed) await operation;
    else await assert.rejects(operation,{recoveryRequired:true});
    assert.equal(cleared,confirmed);
  }
});

test('restart refuses a different journal and zeroes its plaintext',async () => {
  const bytes = Buffer.from('synthetic private snapshot');
  await assert.rejects(resumeCookieImportCleanup({receipt,
    journal:{read:async()=>({receipt:'b'.repeat(64),bytes}),discard:async()=>assert.fail('wrong journal')},
    confirmOutcome:async()=>true,clearPending:async()=>assert.fail('must remain blocked')}),
  {recoveryRequired:true});
  assert.ok(bytes.every(byte=>byte===0));
});

test('restart retries retained journal cleanup before clearing durable state',async () => {
  const calls = [];
  const bytes = Buffer.from('synthetic');
  await resumeCookieImportCleanup({receipt,
    journal:{read:async()=>({receipt,bytes}),discard:async()=>{calls.push('discard');}},
    confirmOutcome:async()=>true,clearPending:async()=>{calls.push('clear');}});
  assert.deepEqual(calls,['discard','clear']);
  assert.ok(bytes.every(byte=>byte===0));
});

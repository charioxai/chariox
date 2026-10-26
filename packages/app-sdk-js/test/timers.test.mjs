import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createTimers } from '../src/timers.js';

test('finishing one invocation cancels only its own timers', async () => {
  let current = 'A', ran = 0;
  const calls = [];
  const timers = createTimers({ waitFor() {
    const owner = current;
    let complete, reject;
    const pending = new Promise((yes, no) => { complete = yes; reject = no; });
    pending.cancel = () => {
      if (owner !== current) return false;
      reject(Error('cancelled')); return true;
    };
    calls.push({ complete, owner });
    return pending;
  } }, error => { throw error; });
  timers.setTimeout(() => { ran++; }, 20);
  current = 'B';
  timers.setTimeout(() => { ran += 100; }, 20);
  await timers.finishInvocation();
  assert.equal(ran, 0);
  calls[0].complete();
  await Promise.resolve();
  assert.equal(ran, 1);
});

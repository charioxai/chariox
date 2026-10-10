import assert from 'node:assert/strict';
import test from 'node:test';
import { revokeDuringInput } from './seat-revocation.mjs';

test('revoke follows observed native mutation while input remains in flight', async () => {
  let selection = 0, cancelled, inputFinished = false;
  const events = [];
  const result = await revokeDuringInput({
    dispatch: () => new Promise((resolve, reject) => { events.push('input'); cancelled = () => { inputFinished = true; reject(Object.assign(new Error('cancelled'), {code:'browser_action_cancelled'})); }; }),
    observe: () => selection, changed: value => value === 126,
    revoke: async () => { assert.equal(inputFinished, false); events.push('revoke'); cancelled(); },
    wait: async () => { selection = 126; },
  });
  assert.equal(result.cancelled, true);
  assert.deepEqual(events, ['input', 'revoke']);
});
test('completed input cannot produce a mid-input revoke PASS', async () => {
  let revoked = false;
  await assert.rejects(revokeDuringInput({ dispatch: async () => true,
    observe: () => 126, changed: () => true, revoke: async () => { revoked = true; }, wait: async () => {} }), /input completed/);
  assert.equal(revoked, false);
});

test('observed mutation plus timeout, transport loss or failed release cannot pass cancellation', async () => {
  for (const message of ['kernel request timeout','transport disconnected','owner-reset-required','helper refused','revoked']) {
    let rejectInput;
    await assert.rejects(revokeDuringInput({dispatch:()=>new Promise((_,reject)=>rejectInput=reject),
      observe:()=>126,changed:()=>true,wait:async()=>{},revoke:async()=>rejectInput(new Error(message))}),/explicit cancellation/);
  }
});

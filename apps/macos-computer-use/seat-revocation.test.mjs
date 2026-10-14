import assert from 'node:assert/strict';
import test from 'node:test';
import { revokeDuringInput, requirePhysicalClick } from './seat-revocation.mjs';

test('revoke follows observed native mutation while input remains in flight', async () => {
  let selection = 0, cancelled, inputFinished = false;
  const events = [];
  const result = await revokeDuringInput({
    dispatch: () => new Promise((resolve, reject) => { events.push('input'); cancelled = () => { inputFinished = true; reject(Object.assign(new Error('cancelled'), {code:'browser_action_cancelled'})); }; }),
    observe: () => selection, changed: value => value === 126,
    revoke: async () => { assert.equal(inputFinished, false); events.push('revoke'); cancelled(); },
    cancelled: async error => error.code === 'browser_action_cancelled',
    wait: async () => { selection = 126; },
  });
  assert.equal(result.cancelled, true);
  assert.deepEqual(events, ['input', 'revoke']);
});
test('completed input cannot produce a mid-input revoke PASS', async () => {
  let revoked = false;
  await assert.rejects(revokeDuringInput({ dispatch: async () => true,
    observe: () => 126, changed: () => true, revoke: async () => { revoked = true; },
    cancelled: async () => true, wait: async () => {} }), /input completed/);
  assert.equal(revoked, false);
});

test('observed mutation plus timeout, transport loss or failed release cannot pass cancellation', async () => {
  for (const message of ['kernel request timeout','transport disconnected','owner-reset-required','helper refused','revoked']) {
    let rejectInput;
    await assert.rejects(revokeDuringInput({dispatch:()=>new Promise((_,reject)=>rejectInput=reject),
      observe:()=>126,changed:()=>true,wait:async()=>{},revoke:async()=>rejectInput(new Error(message)),
      cancelled:async error=>error.code==='user_domain_not_granted'}),/explicit cancellation/);
  }
});

test('a refusal passes only when the caller proves the in-flight Action was cancelled', async () => {
  for (const proven of [true, false]) {
    let rejectInput, seen;
    const run = revokeDuringInput({ dispatch: () => new Promise((_, reject) => rejectInput = reject),
      observe: () => 126, changed: () => true, wait: async () => {},
      revoke: async () => rejectInput(Object.assign(new Error('refused'), { code: 'user_domain_not_granted' })),
      cancelled: async error => { seen = error.code; return proven; } });
    if (proven) assert.equal((await run).cancelled, true);
    else await assert.rejects(run, /explicit cancellation/);
    assert.equal(seen, 'user_domain_not_granted');
  }
  // A completed input is never offered to the cancellation proof.
  let asked = false, resolveInput;
  await assert.rejects(revokeDuringInput({ dispatch: () => new Promise(resolve => resolveInput = resolve),
    observe: () => 126, changed: () => true, wait: async () => {},
    revoke: async () => resolveInput({ dispatched: true }), cancelled: async () => (asked = true) }), /explicit cancellation/);
  assert.equal(asked, false);
});

test('paired clicks accept only the exact CGEventSession receipt', () => {
  const exact = 'dispatched; path=CGEventSession; application completion unproven';
  requirePhysicalClick({ dispatched: true, receipts: [exact] });
  for (const reply of [
    { dispatched: false, receipts: [exact] },
    { dispatched: true, receipts: [] },
    { dispatched: true, receipts: [exact, exact] },
    { dispatched: true, receipts: [exact + ' '] },
    { dispatched: true, receipts: ['dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=126 length=0; point=1,2'] },
    { dispatched: true, receipts: ['dispatched; path=CGEventSession; observed fixture counter increment'] },
    { dispatched: true }, null,
  ]) assert.throws(() => requirePhysicalClick(reply), /paired click receipt/);
});

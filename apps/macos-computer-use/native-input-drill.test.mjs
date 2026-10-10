import test from 'node:test';
import assert from 'node:assert/strict';
import { runNativeInputDrill } from './native-input-drill.mjs';

function hooks() {
  let paused = false, selected = 0;
  return {
    binding: { surface_id: 'fixture', generation: 'one' },
    click: { kind: 'click', x: 10, y: 20, button: 1 }, expectedSelection: 126,
    agentInput: async () => {
      if (paused) throw Object.assign(new Error('physical human paused'), { code: 'browser_action_cancelled' });
      selected = 126;
    },
    selection: async () => selected,
    physicalHuman: async () => { paused = true; return { observed: true, actor_id: 'human:physical-owner' }; },
    ownerRequest: async command => {
      if (command.op === 'release') paused = false;
      if (command.op === 'start') throw new Error('owner-reset-required');
    },
    fatalPress: async () => ({ down_observed: true, helper_identity_verified: true, error: { message: 'owner-reset-required' } }),
    stop: async () => {},
  };
}

test('component oracle accepts explicit physical pause, release and fatal reset receipts', async () => {
  assert.equal((await runNativeInputDrill(hooks())).status, 'PASS');
});
test('timeout does not establish physical pause', async () => {
  const h = hooks(); let count = 0;
  h.agentInput = async () => { if (++count > 1) throw new Error('timeout'); };
  h.selection = async () => 126;
  await assert.rejects(runNativeInputDrill(h), error => error.receipt.status === 'FAIL');
});
test('unverified helper death and generic fatal failure cannot pass', async () => {
  for (const fatal of [{ down_observed: false }, { down_observed: true, helper_identity_verified: true, error: { message: 'timeout' } }]) {
    const h = hooks(); h.fatalPress = async () => fatal;
    await assert.rejects(runNativeInputDrill(h), error => error.receipt.status === 'FAIL');
  }
});
test('owned seat cleanup failure rejects even after successful checks', async () => {
  const h = hooks(); h.stop = async () => { throw new Error('cleanup'); };
  await assert.rejects(runNativeInputDrill(h), error => error.receipt.status === 'FAIL' && error.receipt.checks.at(-1).name === 'owned-seat-cleanup');
});

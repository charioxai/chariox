import test from 'node:test';
import assert from 'node:assert/strict';
import { runNativeInputDrill } from './native-input-drill.mjs';

function hooks() {
  let paused = false, selected = 0;
  return {
    binding: { surface_id: 'fixture', generation: 'one' },
    click: { kind: 'click', x: 10, y: 20, button: 1 }, expectedSelection: 126,
    resumeClick: { kind: 'click', x: 30, y: 20, button: 1 }, expectedResumeSelection: 250,
    agentInput: async command => {
      if (paused) throw Object.assign(new Error('physical human paused'), { code: 'browser_action_cancelled' });
      selected = command.input.x === 30 ? 250 : 126;
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


test('resolved post-release input without a fresh fixture change fails', async () => {
  const h = hooks();
  const dispatch = h.agentInput;
  h.agentInput = async command => {
    if (command.input.x !== h.resumeClick.x) await dispatch(command);
  };
  await assert.rejects(runNativeInputDrill(h), error =>
    error.receipt.checks.some(check => check.name === 'explicit-owner-release-resumes' && !check.passed));
});

test('initial selection already at the click target cannot establish input effect', async () => {
  const h = hooks();
  h.selection = async () => h.expectedSelection;
  await assert.rejects(runNativeInputDrill(h), error => error.receipt.status === 'FAIL');
});


test('missing or reused resume target fails before input dispatch', async () => {
  for (const override of [{ resumeClick: undefined }, { expectedResumeSelection: undefined },
    { expectedResumeSelection: 126 }, { resumeClick: { kind: 'click', x: 10, y: 20, button: 1 } }]) {
    const h = Object.assign(hooks(), override);
    let calls = 0;
    h.agentInput = async () => { calls++; };
    await assert.rejects(runNativeInputDrill(h), error =>
      error.receipt.checks.at(-1).name === 'fresh-click-selection-precondition');
    assert.equal(calls, 0);
  }
});

test('initial acknowledged click without fixture mutation fails', async () => {
  const h = hooks();
  h.agentInput = async () => {};
  await assert.rejects(runNativeInputDrill(h), error =>
    error.receipt.checks.at(-1).name === 'native-click-independent-selection');
});

test('release already at the resume target cannot establish a resumed input effect', async () => {
  const h = hooks();
  const release = h.ownerRequest;
  const selection = h.selection;
  let released = false;
  h.ownerRequest = async command => { await release(command); released ||= command.op === 'release'; };
  h.selection = async () => released ? h.expectedResumeSelection : selection();
  await assert.rejects(runNativeInputDrill(h), error =>
    error.receipt.checks.at(-1).name === 'fresh-resume-selection-precondition');
});


test('fatal recovery rejects a Stop hook that incorrectly clears the press latch', async () => {
  const h = hooks();
  let unresolved = false, stops = 0;
  const owner = h.ownerRequest;
  const fatal = h.fatalPress;
  h.fatalPress = async () => { unresolved = true; return fatal(); };
  h.stop = async () => { stops++; unresolved = false; };
  h.ownerRequest = async command => {
    if (command.op === 'start') {
      if (unresolved) throw new Error('owner-reset-required');
      return;
    }
    return owner(command);
  };
  await assert.rejects(runNativeInputDrill(h), error =>
    error.receipt.checks.at(-1).name === 'stop-start-cannot-clear-fatal-press' &&
    !error.receipt.checks.at(-1).passed);
  assert.equal(stops, 2, 'Stop safety check and final cleanup must both run');
});

test('fatal recovery calls owned Stop before Start and retains final cleanup', async () => {
  const h = hooks();
  const calls = [];
  const owner = h.ownerRequest;
  h.stop = async () => { calls.push('stop'); };
  h.ownerRequest = async command => { calls.push(command.op); return owner(command); };
  assert.equal((await runNativeInputDrill(h)).status, 'PASS');
  assert.deepEqual(calls, ['release', 'stop', 'start', 'stop']);
});

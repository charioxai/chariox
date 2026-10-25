// MP-08 / MP-09 / MP-10 / MP-11: the coordinator's own event wait, against a fake kernel inbox.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { lanewait, replay, formatLine } from './lanewait.mjs';

async function rig(t, keys = ['alpha-r1']) {
  const root = await mkdtemp(join(tmpdir(), 'lanewait-')); t.after(() => rm(root, { recursive: true, force: true }));
  const receipts = join(root, 'loop'), home = join(root, 'watch'), agents = join(root, 'agents');
  await mkdir(receipts); await mkdir(agents);
  const clock = { ms: 10_000_000 };
  const kernel = { subscription: 'inbox-1', targets: [], batches: [], acks: [], calls: [], unsubscribed: [] };
  const launch = key => ({ kernel: 'executor', taskId: key, sessionId: `room-${key}`, operationsSessionId: `ops-${key}`, agentId: `agent-${key}`, attachmentId: 'a' });
  for (const key of keys) await writeFile(join(receipts, `${key}.json`), JSON.stringify({ lane: key, launch: launch(key), input: { deadlineMs: clock.ms + 3_600_000 } }));
  const status = {};
  const options = {
    home, receipts, agents, now: () => clock.ms, sleep: async () => {}, sliceSeconds: 30, settleMs: 0,
    adapter: async (op, input) => {
      assert.equal(op, 'status');
      return { state: 'working', currentPrompt: 'p', lastActivityMs: clock.ms, interactions: [], ...status[input.taskId] };
    },
    cli: async argv => {
      kernel.calls.push(argv.join(' '));
      if (argv[1] === 'subscription') return { subscription: { subscription_id: kernel.subscription, ended: null, targets: kernel.targets } };
      if (argv[1] === 'subscribe') { kernel.targets.push({ kernel: argv[3], session_id: argv[5] }); return {}; }
      if (argv[1] === 'unsubscribe') { kernel.unsubscribed.push(argv[5]); kernel.targets = kernel.targets.filter(x => x.session_id !== argv[5]); return {}; }
      if (argv[1] === 'ack') { kernel.acks.push(Number(argv[2])); return {}; }
      if (argv[1] === 'wait') { clock.ms += Number(argv[3]) * 1000; return kernel.batches.shift() ?? { events: [], timed_out: true, next_cursor: 0 }; }
      throw new Error(`unexpected ${argv}`);
    },
  };
  const event = (key, extra, cursor = 1) => ({ cursor, at_ms: clock.ms, kernel: 'executor', session_id: `room-${key}`, agent_id: `agent-${key}`, ...extra });
  return { root, receipts, home, agents, clock, kernel, status, options, event };
}

test('the inbox follows the running lanes rooms and returns the moment a worker ends its turn', async t => {
  const r = await rig(t, ['alpha-r1', 'beta-r2']);
  await writeFile(join(r.receipts, 'old-r1.json'), JSON.stringify({ lane: 'old-r1', stopped: true, launch: { kernel: 'executor', sessionId: 'room-old', agentId: 'x' } }));
  r.kernel.batches.push({ events: [r.event('alpha-r1', { type: 'agent', summary: 'idle', value: {} })], next_cursor: 5, timed_out: false });
  const lines = await lanewait({ ...r.options, minutes: 10 });
  assert.deepEqual(lines.map(l => [l.lane, l.round, l.kind]), [['alpha', 'r1', 'turn_end']]);
  assert.deepEqual(r.kernel.targets.map(x => x.session_id).sort(), ['ops-alpha-r1', 'ops-beta-r2', 'room-alpha-r1', 'room-beta-r2']);
  assert.deepEqual(r.kernel.acks, [5]);
  assert.match(formatLine(lines[0]), /Z alpha r1 turn_end /);
  assert.equal((await readdir(join(r.home, 'events'))).length, 1, 'the batch is archived before it is acknowledged');
});

test('a routine permission stays with loopd; a critical or credential decision comes to the coordinator once', async t => {
  const r = await rig(t);
  const pending = id => r.event('alpha-r1', { type: 'interaction', refs: { interaction_id: id }, summary: 'Approve Claude Code Bash?', value: { state: 'pending' } });
  r.status['alpha-r1'] = { interactions: [
    { id: 'routine', kind: 'permission', level: 'warning', title: 'Approve Claude Code Bash?', choices: ['allow_once', 'allow_session'] },
    { id: 'critical', kind: 'permission', level: 'critical', title: 'Approve Claude Code Bash?', choices: ['allow_once', 'deny'] }] };
  r.kernel.batches.push({ events: [pending('routine')], next_cursor: 1, timed_out: false });
  r.kernel.batches.push({ events: [{ ...pending('critical'), cursor: 2 }], next_cursor: 2, timed_out: false });
  const first = await lanewait({ ...r.options, minutes: 10 });
  assert.equal(first.length, 1);
  assert.deepEqual([first[0].kind, first[0].interactionId, first[0].reason, first[0].choices], ['decision', 'critical', 'critical', ['allow_once', 'deny']]);
  r.kernel.batches.push({ events: [{ ...pending('critical'), cursor: 3 }], next_cursor: 3, timed_out: false });
  const again = await lanewait({ ...r.options, minutes: 1 });
  assert.deepEqual(again, [], 'a decision already reported is not reported again');
});

test('a removal event ends the lane: one finished line with the round result, and its rooms are unfollowed', async t => {
  const r = await rig(t);
  await mkdir(join(r.agents, 'alpha'));
  await writeFile(join(r.agents, 'alpha', 'LOOP_RUN.json'), JSON.stringify({ round: 'r1', finishedAtMs: 1, finishReason: 'finished', head: 'c'.repeat(40), bundle: '/b' }));
  const rec = JSON.parse(await readFile(join(r.receipts, 'alpha-r1.json'), 'utf8'));
  await writeFile(join(r.receipts, 'alpha-r1.json'), JSON.stringify({ ...rec, stopped: true }));
  r.kernel.batches.push({ events: [r.event('alpha-r1', { type: 'session', summary: 'agent removed', value: { state: 'agent_removed' } })], next_cursor: 9, timed_out: false });
  // The worker was followed while it ran, then stopped by loopd before the removal arrived.
  await mkdir(r.home, { recursive: true });
  await writeFile(join(r.home, 'state.json'), JSON.stringify({ subscriptionId: 'inbox-1', cursor: 0, seq: 0, flags: {}, decisions: {}, tracked: { 'alpha-r1': { startedMs: 1 } }, lastHealthMs: r.clock.ms }));
  const lines = await lanewait({ ...r.options, minutes: 10 });
  assert.equal(lines.length, 1);
  assert.equal(lines[0].kind, 'finished');
  assert.match(lines[0].summary, /^finished head cccccccccc, bundle collected/);
  assert.deepEqual(r.kernel.unsubscribed.sort(), ['ops-alpha-r1', 'room-alpha-r1']);
});

test('a finished round whose removal event was missed is still reported, by the periodic read', async t => {
  const r = await rig(t);
  await mkdir(join(r.agents, 'alpha'));
  await writeFile(join(r.agents, 'alpha', 'LOOP_RUN.json'), JSON.stringify({ round: 'r1', finishedAtMs: 1, finishReason: 'hard_stop' }));
  const lines = await lanewait({ ...r.options, minutes: 10 });
  assert.deepEqual(lines.map(l => [l.kind, l.reason]), [['finished', 'hard_stop']]);
});

test('errors, silence and the round limit are reported once each', async t => {
  const r = await rig(t);
  r.kernel.batches.push({ events: [r.event('alpha-r1', { type: 'agent', summary: 'Provider run failed: Claude Code stopped', value: { reason: 'Provider prompt dispatch failed' } })], next_cursor: 1, timed_out: false });
  const error = await lanewait({ ...r.options, minutes: 10 });
  assert.deepEqual(error.map(l => l.kind), ['error']);
  r.status['alpha-r1'] = { lastActivityMs: r.clock.ms - 20 * 60_000 };
  const silent = await lanewait({ ...r.options, minutes: 10, healthMs: 1000 });
  assert.deepEqual(silent.map(l => l.kind), ['silent']);
  const nothing = await lanewait({ ...r.options, minutes: 2, healthMs: 1000 });
  assert.deepEqual(nothing, [], 'the same flag is not reported again while it lasts');
  r.clock.ms += 4_000_000;
  const long = await lanewait({ ...r.options, minutes: 2, healthMs: 1000 });
  assert.deepEqual(long.map(l => l.kind), ['long_round']);
});

test('a new access grant has a new inbox: the old state is set aside and every lane is followed again', async t => {
  const r = await rig(t);
  await lanewait({ ...r.options, minutes: 1 });
  r.kernel.subscription = 'inbox-2'; r.kernel.targets = [];
  await lanewait({ ...r.options, minutes: 1 });
  assert.deepEqual(r.kernel.targets.map(x => x.session_id).sort(), ['ops-alpha-r1', 'room-alpha-r1']);
  assert.ok((await readdir(r.home)).some(f => /^state\.\d+\.json$/.test(f)));
});

test('events delivered before an interrupted acknowledgement are not printed twice, and replay lists what was emitted', async t => {
  const r = await rig(t);
  const idle = r.event('alpha-r1', { type: 'agent', summary: 'idle', value: {} }, 4);
  r.kernel.batches.push({ events: [idle], next_cursor: 4, timed_out: false });
  const first = await lanewait({ ...r.options, minutes: 1 });
  assert.equal(first.length, 1);
  r.kernel.batches.push({ events: [idle], next_cursor: 4, timed_out: false });
  assert.deepEqual(await lanewait({ ...r.options, minutes: 1 }), []);
  assert.deepEqual((await replay(r.home, 5)).map(l => l.kind), ['turn_end']);
});

test('a gap in the inbox is reported and every lane is read again', async t => {
  const r = await rig(t);
  r.kernel.batches.push({ events: [{ cursor: 2, type: 'gap', at_ms: r.clock.ms }], next_cursor: 2, timed_out: false });
  const lines = await lanewait({ ...r.options, minutes: 10 });
  assert.equal(lines[0].kind, 'attention');
  assert.match(lines[0].summary, /events were lost/);
});

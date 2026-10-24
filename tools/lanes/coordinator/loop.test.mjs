// MP-08 / MP-09 / MP-10 / MP-11: each coordinator step against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { coordinate, save } from './loop.mjs';
import { execute } from '../lanes.mjs';
const head = 'a'.repeat(40);
async function rig(t) {
  const home = await mkdtemp(join(tmpdir(), 'coordinator-')); t.after(() => rm(home, { recursive: true, force: true }));
  const calls = [];
  const config = { kernel: 'executor', providers: { codex: { profile: 'linked', model: 'gpt-6-luna', effort: 'low', permission: 'full' } }, repository: home, bases: { lane: head } };
  const launch = { kernel: 'executor', taskId: 'lane', sessionId: 'room', operationsSessionId: 'ops', agentId: 'agent', attachmentId: 'attached', promptId: 'prompt' };
  const options = { home, config, now: () => 1000, stdin: async () => 'review the exact PR', adapter: async (op, input) => {
    calls.push({ op, input });
    if (op === 'launch') return launch;
    if (op === 'steer') return { ActivePromptSteered: { prompt: { id: 'steer' } } };
    if (op === 'status') return { state: 'idle', currentPrompt: null, lastActivityMs: 1, deadline: { mode: 'kernel', state: 'armed' } };
    if (op === 'fetch') { await mkdir(input.directory, { recursive: true }); await save(join(input.directory, 'ACCEPTANCE_UPDATE.json'), { rows: [{ head }] }); return { fetched: true }; }
    return { stopped: true };
  }, cli: async argv => {
    calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }] } };
    if (argv[1] === 'wait') return { events: [calls.some(c => c.op === 'stop') ? { cursor: 1, kernel: launch.kernel, session_id: launch.sessionId, agent_id: launch.agentId, type: 'session', value: { state: 'agent_removed' } } : { cursor: 1, type: 'agent' }], next_cursor: 1 };
    return {};
  } };
  return { home, options, calls, launch };
}
async function launched(t) { const r = await rig(t); await coordinate('brief', ['lane', '/work', 'codex'], r.options); r.calls.length = 0; return r; }
test('brief: passes input to launch and resumes the exact receipt/deadline', async t => {
  const { options, calls } = await rig(t);
  const one = await coordinate('brief', ['lane', '/work', 'codex'], options);
  options.now = () => 2000;
  const two = await coordinate('brief', ['lane', '/work', 'codex'], options);
  assert.equal(two.input.deadlineMs, one.input.deadlineMs);
  assert.equal(calls.filter(c => c.op === 'launch').length, 2);
  assert.equal(await readFile(one.input.brief, 'utf8'), 'review the exact PR');
  await assert.rejects(coordinate('brief', ['lane', '/other', 'codex'], options), { code: 'launch_conflict' });
});
test('brief: compares piped stdin across fresh coordinator invocations', async t => {
  const r = await rig(t);
  const script = `
    import { coordinate } from ${JSON.stringify(new URL('./loop.mjs', import.meta.url).href)};
    try {
      const result = await coordinate('brief', ['lane', '/work', 'codex'], {
        home: ${JSON.stringify(r.home)}, config: ${JSON.stringify(r.options.config)},
        stdin: async () => { let text = ''; for await (const chunk of process.stdin) text += chunk; return text; },
        adapter: async () => (${JSON.stringify(r.launch)})
      });
      console.log(JSON.stringify(result));
    } catch (error) {
      console.error(JSON.stringify({ error: { code: error.code } })); process.exitCode = 1;
    }
  `;
  const invoke = brief => execute(['--input-type=module', '-e', script], brief, process.execPath);
  const first = await invoke('MP-11: original piped brief\n');
  const resumed = await invoke('MP-11: original piped brief\n');
  assert.deepEqual(resumed, first);
  await assert.rejects(invoke('MP-11: changed piped brief\n'), { code: 'launch_conflict' });
  assert.deepEqual(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')), first);
  assert.equal(await readFile(first.input.brief, 'utf8'), 'MP-11: original piped brief\n');
});
test('watch: one inbox, one WaitEvents, archive before ack, no status polling', async t => {
  const r = await launched(t);
  const cli = r.options.cli;
  r.options.cli = async argv => { if (argv[1] === 'ack') { const { readdir } = await import('node:fs/promises'); assert.equal((await readdir(join(r.home, 'events'))).length, 1); } return cli(argv); };
  await coordinate('watch', ['15'], r.options);
  assert.deepEqual(r.calls.map(c => c.argv?.[1]), ['subscription', 'wait', 'ack']);
  assert.equal(r.calls.filter(c => c.op).length, 0);
});
test('watch: timeout remains a normal event wake and is not acknowledged', async t => {
  const r = await launched(t); const cli = r.options.cli;
  r.options.cli = argv => argv[1] === 'wait' ? { events: [], timed_out: true, next_cursor: 0 } : cli(argv);
  assert.equal((await coordinate('watch', ['1'], r.options)).timed_out, true);
  assert.equal(r.calls.some(c => c.argv?.[1] === 'ack'), false);
});
test('watch: changed grant, missing target and gaps fail clearly', async t => {
  const r = await launched(t); await save(join(r.home, 'subscription.json'), { subscription_id: 'other' });
  await assert.rejects(coordinate('watch', [], r.options), { code: 'subscription_changed' });
  await rm(join(r.home, 'subscription.json'));
  const cli = r.options.cli;
  r.options.cli = argv => argv[1] === 'subscription' ? { subscription: { subscription_id: 'grant', targets: [] } } : cli(argv);
  await assert.rejects(coordinate('watch', [], r.options), { code: 'subscription_target_missing' });
  r.options.cli = argv => argv[1] === 'wait' ? { events: [{ type: 'gap' }], next_cursor: 3 } : cli(argv);
  await assert.rejects(coordinate('watch', [], r.options), { code: 'event_stream_incomplete' });
  assert.equal(r.calls.some(c => c.argv?.[1] === 'ack'), false);
});
test('watch: archives a real exit-3 ending batch and its reason without ack', async t => {
  const r = await launched(t);
  const batch = { events: [{ cursor: 7, type: 'subscription_ended', value: { reason: 'grant_revoked' } }], next_cursor: 7, timed_out: false };
  const calls = join(r.home, 'cli-calls');
  const executable = join(r.home, 'chariox.mjs');
  await writeFile(executable, `#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
const argv = process.argv.slice(2);
appendFileSync(${JSON.stringify(calls)}, JSON.stringify(argv) + '\\n');
if (argv[1] === 'subscription') console.log(JSON.stringify({ subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }] } }));
else if (argv[1] === 'wait') { console.log(JSON.stringify(${JSON.stringify(batch)})); process.exitCode = 3; }
else process.exitCode = 1;
`, { mode: 0o700 });
  const previous = process.env.CHARIOX_BIN;
  process.env.CHARIOX_BIN = executable;
  t.after(() => { if (previous === undefined) delete process.env.CHARIOX_BIN; else process.env.CHARIOX_BIN = previous; });
  const { cli, ...options } = r.options;
  await assert.rejects(coordinate('watch', ['1'], options), { code: 'event_stream_incomplete' });
  const files = await readdir(join(r.home, 'events'));
  assert.equal(files.length, 1);
  assert.deepEqual(JSON.parse(await readFile(join(r.home, 'events', files[0]), 'utf8')), batch);
  assert.deepEqual((await readFile(calls, 'utf8')).trim().split('\n').map(line => JSON.parse(line)[1]), ['subscription', 'wait']);
});
test('deliver: uses current prompt fence and refuses a finished worker', async t => {
  const r = await launched(t); const review = join(r.home, 'review.md'); await writeFile(review, 'check command receipts');
  await assert.rejects(coordinate('deliver', ['lane', review], r.options), { code: 'no_active_prompt' });
  const adapter = r.options.adapter;
  r.options.adapter = (op, input) => op === 'status' ? { currentPrompt: 'current' } : adapter(op, input);
  await coordinate('deliver', ['lane', review], r.options);
  assert.equal(r.calls.at(-1).input.expectedPromptId, 'current');
  assert.equal(r.calls.at(-1).input.prompt, 'check command receipts');
});
test('lanefin: status then fetch then verified bundle; running worker refused', async t => {
  const r = await launched(t); await coordinate('lanefin', ['lane'], r.options);
  assert.deepEqual(r.calls.map(c => c.op), ['status', 'fetch', 'bundle']);
  assert.equal(r.calls.at(-1).input.sessionId, 'ops');
  assert.equal(r.calls.at(-1).input.head, head);
  r.options.adapter = async () => ({ state: 'working', currentPrompt: 'new' });
  await assert.rejects(coordinate('lanefin', ['lane'], r.options), { code: 'lane_still_running' });
});
test('lanehealth: reports deadline and silence without stopping anyone', async t => {
  const r = await launched(t); r.options.now = () => 9_000_000;
  r.options.adapter = async op => { assert.equal(op, 'status'); return { state: 'working', lastActivityMs: 1 }; };
  assert.deepEqual((await coordinate('lanehealth', ['lane'], r.options)).flags, ['LONG-ROUND', 'SILENT']);
});
test('stop: only recorded lane adapter identity; durable stop receipt', async t => {
  const r = await launched(t); await coordinate('stop', ['lane'], r.options);
  assert.deepEqual(r.calls.filter(c => c.op).map(c => c.op), ['stop']);
  assert.equal(r.calls[0].input.agentId, r.launch.agentId);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'))).stopped, true);
  await assert.rejects(coordinate('stop', ['../other'], r.options), { code: 'invalid_lane' });
});

test('watch accepts the Mac max-minutes input and only re-arms WaitEvents', async t => {
  const r = await launched(t); const cli = r.options.cli;
  r.options.cli = async argv => { if (argv[1] === 'wait') { r.calls.push({ argv }); return { events: [], next_cursor: 0, timed_out: true }; } return cli(argv); };
  await coordinate('watch', ['30'], r.options);
  assert.deepEqual(r.calls.map(c => c.argv?.[1]), ['subscription', 'wait', 'wait']);
  assert.deepEqual(r.calls.filter(c => c.argv?.[1] === 'wait').map(c => c.argv.at(-1)), ['900', '900']);
});
test('lanehealth without arguments snapshots every recorded lane once', async t => {
  const r = await launched(t);
  const results = await coordinate('lanehealth', [], r.options);
  assert.equal(results.length, 1); assert.equal(results[0].lane, 'lane');
  assert.deepEqual(r.calls.map(c => c.op), ['status']);
});
test('deliver reuses the confirmed receipt for the same review and current prompt', async t => {
  const r = await launched(t); const review = join(r.home, 'review.md'); await writeFile(review, 'review');
  const adapter = r.options.adapter;
  r.options.adapter = (op, input) => op === 'status' ? { currentPrompt: 'current' } : adapter(op, input);
  await coordinate('deliver', ['lane', review], r.options); await coordinate('deliver', ['lane', review], r.options);
  assert.equal(r.calls.length, 1);
  assert.equal(r.calls[0].input.steerId, JSON.parse(await readFile(join(r.home, 'lane.delivery.json'), 'utf8')).steerId);
});

// MP-08 / MP-10: lane handoffs belong outside the worker checkout.
test('brief and lanefin retain a configured lane directory outside the worktree', async t => {
  const r = await rig(t);
  r.options.config.laneDirectories = { lane: '/lane-state/lane' };
  const started = await coordinate('brief', ['lane', '/work', 'codex'], r.options);
  assert.equal(started.input.remoteBrief, '/lane-state/lane/ROUND_BRIEF.md');
  assert.equal(started.laneDirectory, '/lane-state/lane');
  await coordinate('lanefin', ['lane'], r.options);
  assert.equal(r.calls.find(c => c.op === 'fetch').input.worktree, '/lane-state/lane');
  assert.equal(r.calls.find(c => c.op === 'bundle').input.worktree, '/work');
  assert.equal(r.calls.find(c => c.op === 'bundle').input.remoteBundle, '/lane-state/lane/round.bundle');
});
test('brief refuses relative lane directories before launch', async t => {
  const r = await rig(t);
  r.options.config.laneDirectories = { lane: 'relative' };
  await assert.rejects(coordinate('brief', ['lane', '/work', 'codex'], r.options), { code: 'invalid_lane_directory' });
  assert.equal(r.calls.some(c => c.op === 'launch'), false);
});

test('lanefin collects the bundle when acceptance merge already consumed the update', async t => {
  const r = await launched(t), adapter = r.options.adapter;
  r.options.adapter = async (op, input) => {
    if (op === 'fetch') { r.calls.push({ op, input }); return { 'ACCEPTANCE_UPDATE.json': { missing: true, reason: 'file_not_found' } }; }
    if (op === 'bundle') { r.calls.push({ op, input }); assert.equal(input.head, undefined); return { head }; }
    return adapter(op, input);
  };
  const result = await coordinate('lanefin', ['lane'], r.options);
  assert.equal(result.bundle.head, head);
  assert.deepEqual(r.calls.map(c => c.op), ['status', 'fetch', 'bundle']);
});

test('stop archives exact removal before ack and reuses the confirmed receipt', async t => {
  const r = await launched(t);
  const event = { cursor: 17, kernel: r.launch.kernel, session_id: r.launch.sessionId, agent_id: r.launch.agentId, type: 'session', value: { state: 'agent_removed' } };
  const cli = r.options.cli;
  r.options.cli = async argv => {
    if (argv[1] === 'wait') { r.calls.push({ argv }); return { events: [event], next_cursor: 17 }; }
    if (argv[1] === 'ack') {
      const saved = JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8'));
      assert.deepEqual(saved.stop.removal.event, event);
      assert.equal((await readdir(join(r.home, 'events'))).length, 1);
    }
    return cli(argv);
  };
  const result = await coordinate('stop', ['lane'], r.options);
  assert.deepEqual(result.removal.event, event);
  const count = r.calls.length;
  assert.deepEqual(await coordinate('stop', ['lane'], r.options), result);
  assert.equal(r.calls.length, count);
});
test('stop retains its physical receipt when the event stream ends without ack', async t => {
  const r = await launched(t), cli = r.options.cli;
  r.options.cli = async argv => argv[1] === 'wait' ? { events: [{ cursor: 1, type: 'subscription_ended', value: { reason: 'grant_revoked' } }], next_cursor: 1 } : cli(argv);
  await assert.rejects(coordinate('stop', ['lane'], r.options), { code: 'event_stream_incomplete' });
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).stopped, true);
  assert.equal(r.calls.some(c => c.argv?.[1] === 'ack'), false);
});
test('stop does not confirm another worker removal and reports a bounded timeout', async t => {
  const r = await launched(t), cli = r.options.cli;
  r.options.cli = async argv => argv[1] === 'wait' ? { events: [{ cursor: 1, kernel: r.launch.kernel, session_id: r.launch.sessionId, agent_id: 'another', type: 'session', value: { state: 'agent_removed' } }], next_cursor: 1, timed_out: true } : cli(argv);
  await assert.rejects(coordinate('stop', ['lane'], r.options), { code: 'stop_confirmation_timeout' });
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).stopped, true);
});
test('stop retry confirms a removal another consumer archived and acknowledged', async t => {
  const r = await launched(t), cli = r.options.cli;
  const event = { cursor: 9, kernel: r.launch.kernel, session_id: r.launch.sessionId, agent_id: r.launch.agentId, type: 'session', value: { state: 'agent_removed' } };
  r.options.cli = async argv => argv[1] === 'wait' ? { events: [], next_cursor: 0, timed_out: true } : cli(argv);
  await assert.rejects(coordinate('stop', ['lane'], r.options), { code: 'stop_confirmation_timeout' });
  await save(join(r.home, 'events', '1000-other-consumer.json'), { events: [event], next_cursor: 9 });
  const waits = r.calls.filter(c => c.argv?.[1] === 'wait').length, stops = r.calls.filter(c => c.op === 'stop').length;
  const result = await coordinate('stop', ['lane'], r.options);
  assert.deepEqual(result.removal.event, event);
  assert.equal(r.calls.filter(c => c.argv?.[1] === 'wait').length, waits);
  assert.equal(r.calls.filter(c => c.op === 'stop').length, stops);
});

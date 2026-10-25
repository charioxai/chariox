// MP-08 / MP-09 / MP-10 / MP-11: each coordinator step against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile, readdir, copyFile, rename } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { coordinate, save } from './loop.mjs';
import { execute, run, LaneError } from '../lanes.mjs';
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
    if (op === 'fetch') { await mkdir(input.directory, { recursive: true }); await save(join(input.directory, 'ACCEPTANCE_UPDATE.json'), { rows: [{ head }] }); return { 'ACCEPTANCE_UPDATE.json': { acceptance: { valid: true } } }; }
    return { stopped: true };
  }, cli: async argv => {
    calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }, { kernel: 'executor', session_id: 'room', agent_id: 'agent' }, { kernel: 'executor', session_id: 'ops' }] } };
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
  assert.deepEqual(await readdir(join(r.home, 'events')).catch(() => []), [], 'an empty wake leaves no archive file');
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
if (argv[1] === 'subscription') console.log(JSON.stringify({ subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }, { kernel: 'executor', session_id: 'room', agent_id: 'agent' }, { kernel: 'executor', session_id: 'ops' }] } }));
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
  assert.equal(r.calls.find(c => c.op).input.agentId, r.launch.agentId);
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
test('deliver treats prompt_steer_event_unrecorded as delivered and never resends', async t => {
  const r = await launched(t), adapter = r.options.adapter, review = join(r.home, 'review.md');
  await writeFile(review, 'please change X');
  r.options.adapter = async (op, input) => {
    if (op === 'steer') { r.calls.push({ op, input }); throw new LaneError('prompt_steer_event_unrecorded'); }
    if (op === 'status') return { ...(await adapter(op, input)), currentPrompt: 'prompt', state: 'working', capabilities: ['outside_steer_events_v1'] };
    return adapter(op, input);
  };
  const first = await coordinate('deliver', ['lane', review], r.options);
  assert.equal(first.confirmation.mode, 'delivered-event-unrecorded');
  const again = await coordinate('deliver', ['lane', review], r.options);
  assert.equal(again.confirmation.mode, 'delivered-event-unrecorded');
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1);
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

test('lanefin bundles the worktree head when the worker wrote a free-form acceptance head', async t => {
  const r = await launched(t), adapter = r.options.adapter;
  const issues = [{ field: 'rows[0].head', problem: 'must be one full 40-character git commit id' }];
  r.options.adapter = async (op, input) => {
    if (op === 'fetch') { r.calls.push({ op, input }); return { 'ACCEPTANCE_UPDATE.json': { path: 'x', sha256: 'y', acceptance: { valid: false, issues } } }; }
    if (op === 'bundle') { r.calls.push({ op, input }); assert.equal(input.head, undefined); return { head }; }
    return adapter(op, input);
  };
  const result = await coordinate('lanefin', ['lane'], r.options);
  assert.equal(result.bundle.head, head);
  assert.deepEqual(result.acceptanceIssues, issues);
});

test('lanefin bundles after the real dogfood r9 acceptance files through the real fetch adapter', async t => {
  for (const lane of ['amint', 'dictation']) {
    const r = await launched(t), adapter = r.options.adapter;
    const fixture = new URL(`../fixtures/r9-${lane}-ACCEPTANCE_UPDATE.json`, import.meta.url).pathname;
    r.options.adapter = async (op, input) => {
      if (op === 'fetch') return run('fetch', input, { execute: async argv => { if (argv.at(-2).endsWith('.json')) await copyFile(fixture, argv.at(-1)); else await writeFile(argv.at(-1), 'notes'); return {}; } });
      if (op === 'bundle') { r.calls.push({ op, input }); return { head: 'd'.repeat(40), sha256: 'e'.repeat(64), path: 'p' }; }
      return adapter(op, input);
    };
    const result = await coordinate('lanefin', ['lane'], r.options);
    assert.equal(result.bundle.head, 'd'.repeat(40), lane);
    assert.equal(r.calls.find(c => c.op === 'bundle').input.head, undefined);
    assert.equal(result.acceptanceIssues[0].field, 'rows[0].head');
  }
});

test('stop drops the agent-level event target before the agent is destroyed', async t => {
  const r = await launched(t);
  await coordinate('stop', ['lane'], r.options);
  const unsubscribe = r.calls.findIndex(c => c.argv?.[1] === 'unsubscribe'), destroy = r.calls.findIndex(c => c.op === 'stop');
  assert.ok(unsubscribe >= 0 && unsubscribe < destroy);
  assert.deepEqual(r.calls[unsubscribe].argv, ['events', 'unsubscribe', '--kernel', 'executor', '--session', 'room', '--agent', 'agent']);
});

test('opening the inbox drops the targets of a stopped worker (sessions once removal is confirmed) and refuses a stream the kernel still refuses', async t => {
  const r = await launched(t);
  const record = JSON.parse(await readFile(join(r.home, 'lane.json')));
  await save(join(r.home, 'lane.json'), { ...record, stopped: true, stop: { stopped: true, removal: { event: {} } } });
  let refused = true;
  const targets = [{ kernel: 'executor', session_id: 'room' }, { kernel: 'executor', session_id: 'room', agent_id: 'agent' }, { kernel: 'executor', session_id: 'ops' }, { kernel: 'executor', session_id: 'other' }];
  const cli = argv => {
    r.calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: targets.filter(t => !r.calls.some(c => c.argv?.[1] === 'unsubscribe' && c.argv[5] === t.session_id && (c.argv[7] ?? undefined) === (t.agent_id ?? undefined))), kernels: refused ? [{ kernel: 'executor', online: false, error: { code: 'events_refused', message: 'event target agent was not found' } }] : [{ kernel: 'executor', online: true }] } };
    return {};
  };
  r.options.cli = cli; r.options.settleMs = 0;
  await assert.rejects(coordinate('watch', ['1'], r.options), { code: 'subscription_unhealthy' });
  const looks = r.calls.filter(c => c.argv?.[1] === 'subscription').length;
  assert.ok(looks >= 5, 'it looks again before giving up');
  const dropped = r.calls.filter(c => c.argv?.[1] === 'unsubscribe').map(c => c.argv.slice(4).join(' '));
  assert.deepEqual(dropped, ['--session room --agent agent', '--session room', '--session ops']);
  assert.equal(dropped.some(d => d.includes('other')), false);
  refused = false;
  r.options.cli = argv => argv[1] === 'wait' ? { events: [], timed_out: true, next_cursor: 0 } : cli(argv);
  assert.equal((await coordinate('watch', ['1'], r.options)).timed_out, true);
});

test('opening the inbox waits for the kernel to clear its refusal after the dead target goes', async t => {
  const r = await launched(t);
  const record = JSON.parse(await readFile(join(r.home, 'lane.json')));
  await save(join(r.home, 'lane.json'), { ...record, stopped: true, stop: { stopped: true } });
  let looks = 0;
  r.options.settleMs = 0;
  r.options.cli = argv => {
    r.calls.push({ argv });
    if (argv[1] === 'subscription') {
      const gone = r.calls.some(c => c.argv?.[1] === 'unsubscribe');
      return { subscription: { subscription_id: 'grant', targets: gone ? [] : [{ kernel: 'executor', session_id: 'room', agent_id: 'agent' }], kernels: gone && ++looks >= 3 ? [{ kernel: 'executor', online: true }] : [{ kernel: 'executor', online: false, error: { code: 'events_refused', message: 'event target agent was not found' } }] } };
    }
    return argv[1] === 'wait' ? { events: [], timed_out: true, next_cursor: 0 } : {};
  };
  assert.equal((await coordinate('watch', ['1'], r.options)).timed_out, true);
  assert.equal(looks, 3);
});

test('opening the inbox keeps the session targets of a stopped worker whose removal was not seen yet', async t => {
  const r = await launched(t);
  const record = JSON.parse(await readFile(join(r.home, 'lane.json')));
  await save(join(r.home, 'lane.json'), { ...record, stopped: true, stop: { stopped: true } });
  const cli = argv => {
    r.calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }, { kernel: 'executor', session_id: 'room', agent_id: 'agent' }], kernels: [{ kernel: 'executor', online: true }] } };
    return argv[1] === 'wait' ? { events: [], timed_out: true, next_cursor: 0 } : {};
  };
  r.options.cli = cli;
  await coordinate('watch', ['1'], r.options);
  assert.deepEqual(r.calls.filter(c => c.argv?.[1] === 'unsubscribe').map(c => c.argv.slice(4).join(' ')), ['--session room --agent agent']);
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

test('lanehealth flags a worker in error', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'status' ? { state: 'error', currentPrompt: null, lastActivityMs: 1 } : adapter(op, input);
  assert.deepEqual((await coordinate('lanehealth', ['lane'], r.options)).flags, ['ERROR']);
});

test('steer: a running turn is steered with the kernel answer as the only confirmation, and the section is recorded once', async t => {
  const r = await launched(t);
  const status = r.options.adapter;
  r.options.adapter = async (op, input) => { if (op === 'status') return { state: 'working', currentPrompt: 'turn-1', lastActivityMs: 1 }; return status(op, input); };
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nchange X\n');
  const result = await coordinate('steer', ['lane', file], r.options);
  assert.equal(result.mode, 'steered');
  const steer = r.calls.find(c => c.op === 'steer');
  assert.equal(steer.input.expectedPromptId, 'turn-1');
  assert.match(steer.input.steerId, /^review-[0-9a-f]{64}$/, 'the id loopd would use for the same text');
  assert.equal(r.calls.some(c => c.argv?.[0] === 'events'), false, 'no inbox is read');
  const again = await coordinate('steer', ['lane', file], r.options);
  assert.equal(again.repeated, true);
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1, 'the same section is never sent twice');
});

test('steer: the result is one short line (lane, mode, prompt id, agent status), never the kernel session or the note text', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  const prompt = { id: 'prompt-42', status: 'Running', prompt: 'the whole note text', target_agent_id: 'agent' };
  r.options.adapter = async (op, input) => op === 'status' ? { state: 'working', currentPrompt: 'turn-1', lastActivityMs: 1 } : op === 'steer' ? { ActivePromptSteered: { prompt } }
    : op === 'prompt' ? { PromptSubmitted: { outcome: { Started: { prompt } }, session: { id: 's', blob: 'x'.repeat(5000) } } } : adapter(op, input);
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nthe whole note text\n');
  const steered = await coordinate('steer', ['lane', file], r.options);
  assert.deepEqual(steered, { lane: 'lane', mode: 'steered', promptId: 'prompt-42', agentStatus: 'Running' });
  assert.deepEqual(await coordinate('steer', ['lane', file], r.options), { ...steered, repeated: true });
  const idle = await launched(t); const idleAdapter = idle.options.adapter;
  idle.options.adapter = async (op, input) => op === 'prompt' ? { PromptSubmitted: { outcome: { Started: { prompt } }, session: { id: 's', blob: 'x'.repeat(5000) } } } : idleAdapter(op, input);
  const next = join(idle.home, 'note.md'); await writeFile(next, '## 10:00Z coordinator\nnext\n');
  const started = await coordinate('steer', ['lane', next], idle.options);
  assert.deepEqual(started, { lane: 'lane', mode: 'prompt-started', promptId: 'prompt-42', agentStatus: 'Running' });
  assert.ok(JSON.stringify(started).length < 200);
});

test('steer: a worker between turns gets the text as its next prompt', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'prompt' ? (r.calls.push({ op, input }), { PromptSubmitted: { outcome: { Started: { prompt: { id: 'next' } } } } }) : adapter(op, input);
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nnext thing\n');
  const result = await coordinate('steer', ['lane', file], r.options);
  assert.equal(result.mode, 'prompt-started');
  assert.equal(r.calls.filter(c => c.op === 'prompt').length, 1);
  assert.match(r.calls.find(c => c.op === 'prompt').input.commandId, /^direct-[0-9a-f]{64}$/);
  assert.equal(r.calls.some(c => c.op === 'steer'), false);
});

test('steer: a turn that ended between the read and the steer is read again once, then the note becomes the next prompt', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter; let reads = 0;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return reads++ === 0 ? { state: 'working', currentPrompt: 'turn-1', lastActivityMs: 1 } : { state: 'idle', currentPrompt: null, lastActivityMs: 1 };
    if (op === 'steer') throw new LaneError('prompt_steer_rejected', 'stale');
    if (op === 'prompt') return { PromptSubmitted: { outcome: { Queued: { prompt: { id: 'q' } } } } };
    return adapter(op, input);
  };
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nlate\n');
  assert.equal((await coordinate('steer', ['lane', file], r.options)).mode, 'prompt-queued');
});

test('steer: a kernel that took the note but could not record its event counts as delivered', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return { state: 'working', currentPrompt: 'turn-1', lastActivityMs: 1 };
    if (op === 'steer') throw new LaneError('prompt_steer_event_unrecorded', 'delivered');
    return adapter(op, input);
  };
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nx\n');
  assert.equal((await coordinate('steer', ['lane', file], r.options)).mode, 'steered');
});

test('answer: a pending decision is answered with an offered choice; anything else is refused with what is pending or offered', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return { state: 'working', currentPrompt: 'p', lastActivityMs: 1, interactions: [{ id: 'ask-1', kind: 'choice', choices: ['red', 'blue'] }] };
    if (op === 'respond') { r.calls.push({ op, input }); return {}; }
    return adapter(op, input);
  };
  assert.deepEqual(await coordinate('answer', ['lane', 'ask-1', 'blue'], r.options), { lane: 'lane', decision: 'ask-1', choice: 'blue', answered: true });
  assert.equal(r.calls.find(c => c.op === 'respond').input.choiceId, 'blue');
  await assert.rejects(coordinate('answer', ['lane', 'ask-1', 'green'], r.options), { code: 'choice_not_offered', message: /red, blue/ });
  await assert.rejects(coordinate('answer', ['lane', 'gone', 'red'], r.options), { code: 'decision_not_pending', message: /ask-1/ });
  assert.equal(r.calls.filter(c => c.op === 'respond').length, 1);
});

test('steer and answer take the lane name and use the round loopd is running; a lane with no running round is refused by name', async t => {
  const r = await launched(t);
  await rename(join(r.home, 'lane.json'), join(r.home, 'lane-r4.json'));
  const receipt = JSON.parse(await readFile(join(r.home, 'lane-r4.json'), 'utf8'));
  await writeFile(join(r.home, 'lane-r4.json'), JSON.stringify({ ...receipt, lane: 'lane-r4' }));
  const agents = join(r.home, 'agents'); await mkdir(join(agents, 'lane'), { recursive: true });
  await writeFile(join(agents, 'lane', 'LOOP_RUN.json'), JSON.stringify({ round: 'r4', launchedAtMs: 1 }));
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'status' ? { state: 'working', currentPrompt: 'turn', lastActivityMs: 1, interactions: [{ id: 'd', choices: ['yes'] }] } : adapter(op, input);
  const options = { ...r.options, agents };
  assert.equal((await coordinate('answer', ['lane', 'd', 'yes'], options)).answered, true);
  await writeFile(join(agents, 'lane', 'LOOP_RUN.json'), JSON.stringify({ round: 'r4', launchedAtMs: 1, finishedAtMs: 2 }));
  await assert.rejects(coordinate('answer', ['lane', 'd', 'yes'], options), { code: 'lane_not_running' });
  await assert.rejects(coordinate('answer', ['nobody', 'd', 'yes'], options), { code: 'lane_not_running' });
});

test('a stopped classic receipt of the same lane name does not capture steer and answer while a round is running', async t => {
  const r = await launched(t);
  const receipt = JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8'));
  await writeFile(join(r.home, 'lane-r4.json'), JSON.stringify({ ...receipt, lane: 'lane-r4' }));
  await writeFile(join(r.home, 'lane.json'), JSON.stringify({ ...receipt, stopped: true, launch: { ...receipt.launch, agentId: 'old-agent', taskId: 'old' } }));
  const agents = join(r.home, 'agents'); await mkdir(join(agents, 'lane'), { recursive: true });
  await writeFile(join(agents, 'lane', 'LOOP_RUN.json'), JSON.stringify({ round: 'r4', launchedAtMs: 1 }));
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'status' ? { state: 'working', currentPrompt: 'turn', lastActivityMs: 1, interactions: [{ id: 'd', choices: ['yes'] }] } : adapter(op, input);
  const options = { ...r.options, agents };
  await coordinate('answer', ['lane', 'd', 'yes'], options);
  const note = join(r.home, 'note.md'); await writeFile(note, '## 10:00Z coordinator\nwhere am I\n');
  await coordinate('steer', ['lane', note], options);
  const used = r.calls.filter(c => ['respond', 'steer', 'status'].includes(c.op)).map(c => c.input.agentId);
  assert.ok(used.length === 2 && used.every(id => id === receipt.launch.agentId), `every call targets the running round, saw ${used}`);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).stopped, true);
});

test('steer waits for a collection in progress, then refuses a lane whose round is over instead of prompting a stopped worker', async t => {
  const r = await launched(t);
  const { tryLaneLock, releaseLaneLock } = await import('./lane-lock.mjs');
  const lock = await tryLaneLock(r.home, 'lane', 'loopd');
  assert.ok(lock);
  assert.equal(await tryLaneLock(r.home, 'lane', 'steer'), undefined, 'a second holder is refused while the lock is fresh');
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\nlate note\n');
  const waiting = coordinate('steer', ['lane', file], r.options);
  await new Promise(resolve => setTimeout(resolve, 400));
  assert.equal(r.calls.some(c => c.op === 'prompt' || c.op === 'steer'), false, 'nothing is admitted while the lane is being collected');
  await coordinate('stop', ['lane'], r.options);
  await releaseLaneLock(lock);
  await assert.rejects(waiting, { code: 'lane_not_running' });
  assert.equal(r.calls.some(c => c.op === 'prompt' || c.op === 'steer'), false);
});

test('steer: a prompt the kernel accepted but whose reply was lost is replayed under the same command id, never as a new steer', async t => {
  const r = await launched(t);
  const adapter = r.options.adapter;
  let accepted = false, lost = true;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return accepted ? { state: 'working', currentPrompt: 'prompt-from-note', lastActivityMs: 1 } : { state: 'idle', currentPrompt: null, lastActivityMs: 1 };
    if (op === 'prompt') { r.calls.push({ op, input }); accepted = true; if (lost) { lost = false; throw new LaneError('local_transport_error', 'reply lost'); } return { PromptSubmitted: { outcome: { Started: { prompt: { id: 'prompt-from-note', status: 'Running' } } } } }; }
    return adapter(op, input);
  };
  const file = join(r.home, 'note.md'); await writeFile(file, '## 10:00Z coordinator\ndo X\n');
  await assert.rejects(coordinate('steer', ['lane', file], r.options), { code: 'local_transport_error' });
  const saved = JSON.parse(await readFile(join(r.home, 'lane.direct.json'), 'utf8'));
  assert.equal(Object.values(saved.pendingDeliveries)[0].operation, 'prompt', 'the operation is saved before it is sent');
  const retried = await coordinate('steer', ['lane', file], r.options);
  assert.equal(retried.mode, 'prompt-started');
  const prompts = r.calls.filter(c => c.op === 'prompt');
  assert.equal(prompts.length, 2);
  assert.equal(prompts[1].input.commandId, prompts[0].input.commandId);
  assert.equal(r.calls.some(c => c.op === 'steer'), false, 'the now-running turn is not steered with the same text');
  const ledger = JSON.parse(await readFile(join(r.home, 'lane.direct.json'), 'utf8'));
  assert.deepEqual(ledger.pendingDeliveries, {});
  assert.equal(ledger.sections.length, 1);
});

test('steer: unresolved A survives B and a new driver process replays A exactly once', async t => {
  const r = await launched(t);
  const world = join(r.home, 'world.json'); await writeFile(world, JSON.stringify({ accepted: {}, lose: true }));
  const script = `
    import { coordinate } from ${JSON.stringify(new URL('./loop.mjs', import.meta.url).href)};
    import { LaneError } from ${JSON.stringify(new URL('../lanes.mjs', import.meta.url).href)};
    import { readFile, writeFile } from 'node:fs/promises';
    const path = ${JSON.stringify(world)};
    const state = JSON.parse(await readFile(path, 'utf8'));
    try {
      const result = await coordinate('steer', ['lane', process.argv[1]], {
        home: ${JSON.stringify(r.home)}, config: ${JSON.stringify(r.options.config)},
        adapter: async (op, input) => {
          if (op === 'status') return { state: Object.keys(state.accepted).length ? 'working' : 'idle', currentPrompt: Object.keys(state.accepted).length ? 'A-prompt' : null };
          if (op !== 'prompt' && op !== 'steer') throw new Error('unexpected ' + op);
          state.accepted[input.commandId ?? input.steerId] = input.prompt;
          const lose = state.lose; state.lose = false;
          await writeFile(path, JSON.stringify(state));
          if (lose) throw new LaneError('local_transport_error', 'A reply lost');
          return op === 'prompt' ? { PromptSubmitted: { outcome: { Started: { prompt: { id: 'A-prompt' } } } } } : { ActivePromptSteered: { prompt: { id: 'B-steer' } } };
        }
      });
      console.log(JSON.stringify(result));
    } catch (error) { console.error(JSON.stringify({ error: { code: error.code } })); process.exitCode = 1; }
  `;
  const a = join(r.home, 'A.md'), b = join(r.home, 'B.md');
  await writeFile(a, '## A\nDo A\n'); await writeFile(b, '## B\nDo B\n');
  const invoke = path => execute(['--input-type=module', '-e', script, path], undefined, process.execPath);
  await assert.rejects(invoke(a), { code: 'local_transport_error' });
  await invoke(b); await invoke(a);
  const { accepted } = JSON.parse(await readFile(world, 'utf8'));
  assert.deepEqual(Object.values(accepted).sort(), ['## A\nDo A\n', '## B\nDo B\n']);
  assert.equal(Object.keys(accepted).length, 2, 'A never acquires a fresh steer identity across process restarts');
});

test('retiring a lost Room removes its targets so another lane can watch and answer permissions', async t => {
  const r = await launched(t);
  const healthy = { ...r.launch, sessionId: 'healthy-room', operationsSessionId: 'healthy-ops', agentId: 'healthy-agent' };
  await save(join(r.home, 'healthy.json'), { lane: 'healthy', input: {}, launch: healthy });
  const targets = [r.launch, healthy].flatMap(l => [
    { kernel: l.kernel, session_id: l.sessionId },
    { kernel: l.kernel, session_id: l.sessionId, agent_id: l.agentId },
    { kernel: l.kernel, session_id: l.operationsSessionId },
  ]);
  const permission = { id: 'healthy-permission', kind: 'permission', choices: ['allow_once'] };
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => {
    if (op === 'status') {
      if (input.agentId === r.launch.agentId) throw new LaneError('session_not_found');
      return { state: 'working', currentPrompt: 'healthy-prompt', interactions: [permission] };
    }
    if (op === 'fetch') return {};
    return adapter(op, input);
  };
  r.options.cli = async (argv, body) => {
    r.calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets,
      kernels: targets.some(t => t.session_id === 'room') ? [{ kernel: 'executor', error: { code: 'events_refused', message: 'Room no longer exists' } }] : [] } };
    if (argv[1] === 'unsubscribe') {
      const session = argv[argv.indexOf('--session') + 1];
      const agent = argv.includes('--agent') ? argv[argv.indexOf('--agent') + 1] : undefined;
      const index = targets.findIndex(t => t.session_id === session && t.agent_id === agent);
      if (index >= 0) targets.splice(index, 1);
    }
    if (argv[0] === 'request') return { SessionState: { agents: [{ id: healthy.agentId }] } };
    if (argv[1] === 'wait') return { events: [{ type: 'interaction', session_id: healthy.sessionId, agent_id: healthy.agentId, refs: { interaction_id: permission.id } }], next_cursor: 1 };
    return {};
  };
  await coordinate('lanefin', ['lane', 'worker_lost'], r.options);
  const stopped = JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8'));
  assert.equal(stopped.stop.workerLost, true);
  assert.equal(stopped.stop.removal, undefined, 'a removed Room cannot emit a removal receipt');
  const batch = await coordinate('watch', ['1'], { ...r.options, settleMs: 0 });
  assert.equal(batch.events[0].agent_id, healthy.agentId);
  await coordinate('answer', ['healthy', permission.id, 'allow_once'], r.options);
  assert.ok(r.calls.some(c => c.op === 'respond' && c.input.interactionId === permission.id));
  assert.equal(targets.some(t => t.session_id === 'room'), false);
  assert.equal(targets.filter(t => t.session_id === 'healthy-room').length, 2);
});

for (const operation of ['steer', 'deliver']) test(`${operation}: renews a stale attachment once and persists it for the next invocation`, async t => {
  const r = await launched(t);
  const review = join(r.home, 'review.md'); await writeFile(review, '## MP-08 / MP-09 / MP-10 / MP-11\nrenew the attachment\n');
  const original = r.options.adapter, attempts = [];
  let renewals = 0;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return { currentPrompt: 'current' };
    if (op === 'launch') { renewals++; return { ...r.launch, attachmentId: 'renewed' }; }
    if (op === 'steer') {
      attempts.push(input);
      if (input.attachmentId !== 'renewed') throw new LaneError('attachment_not_found');
    }
    return original(op, input);
  };
  await coordinate(operation, ['lane', review], r.options);
  assert.deepEqual(attempts.map(a => a.attachmentId), ['attached', 'renewed']);
  assert.notEqual(attempts[0].steerId, attempts[1].steerId, 'a definitive refusal needs a new command for its changed attachment body');
  assert.equal(renewals, 1);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).launch.attachmentId, 'renewed');
  await writeFile(review, '## MP-08 / MP-09 / MP-10 / MP-11\na second note\n');
  await coordinate(operation, ['lane', review], r.options);
  assert.equal(attempts.at(-1).attachmentId, 'renewed');
});

for (const operation of ['steer', 'deliver']) test(`${operation}: two attachment refusals fail with a typed error and retain the operation for replay`, async t => {
  const r = await launched(t);
  const review = join(r.home, 'review.md'); await writeFile(review, 'MP-08 / MP-09 / MP-10 / MP-11: expired twice');
  let launches = 0, sends = 0;
  r.options.adapter = async op => {
    if (op === 'status') return { currentPrompt: 'current' };
    if (op === 'launch') { launches++; return { ...r.launch, attachmentId: 'renewed' }; }
    if (op === 'steer') { sends++; throw new LaneError('attachment_not_found'); }
    throw new Error(`unexpected ${op}`);
  };
  await assert.rejects(coordinate(operation, ['lane', review], r.options), { code: 'attachment_recovery_failed' });
  assert.equal(launches, 1); assert.equal(sends, 2);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).launch.attachmentId, 'renewed');
  if (operation === 'steer') assert.equal(Object.keys(JSON.parse(await readFile(join(r.home, 'lane.direct.json'), 'utf8')).pendingDeliveries).length, 1);
});

test('steer: renews an idle worker attachment without changing its pending prompt command', async t => {
  const r = await launched(t);
  const review = join(r.home, 'review.md'); await writeFile(review, 'MP-08 / MP-09 / MP-10 / MP-11: next turn');
  const sent = []; let renewals = 0;
  r.options.adapter = async (op, input) => {
    if (op === 'status') return { currentPrompt: null };
    if (op === 'launch') { renewals++; return { ...r.launch, attachmentId: 'renewed' }; }
    if (op === 'prompt') {
      sent.push(input);
      if (input.attachmentId === 'attached') throw new LaneError('attachment_not_found');
      return { PromptSubmitted: { outcome: { Started: { prompt: { id: 'next', status: 'running' } } } } };
    }
    throw new Error(`unexpected ${op}`);
  };
  assert.equal((await coordinate('steer', ['lane', review], r.options)).mode, 'prompt-started');
  assert.equal(renewals, 1);
  assert.notEqual(sent[0].commandId, sent[1].commandId);
  const record = JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8'));
  assert.equal(record.launch.attachmentId, 'renewed'); assert.equal(record.latestPromptId, 'next');
  await coordinate('brief', ['lane', '/work', 'codex'], r.options);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8')).launch.attachmentId, 'renewed');
});

for (const operation of ['steer', 'deliver']) test(`${operation}: uncertain delivery after renewal replays the exact saved command and attachment`, async t => {
  const r = await launched(t);
  const review = join(r.home, 'review.md'); await writeFile(review, 'MP-08 / MP-09 / MP-10 / MP-11: uncertain renewal');
  let renewals = 0, accepted, loseReply = true;
  const commands = new Map();
  r.options.adapter = async (op, input) => {
    if (op === 'status') return { currentPrompt: 'current' };
    if (op === 'launch') { renewals++; return { ...r.launch, attachmentId: 'renewed' }; }
    if (op !== 'steer') throw new Error(`unexpected ${op}`);
    const body = JSON.stringify(input);
    if (commands.has(input.steerId) && commands.get(input.steerId) !== body) throw new LaneError('duplicate_command_conflict');
    commands.set(input.steerId, body);
    if (input.attachmentId === 'attached') throw new LaneError('attachment_not_found');
    if (!accepted) accepted = input;
    else assert.deepEqual(input, accepted, 'an uncertain command must keep the entire original body');
    if (loseReply) { loseReply = false; throw new LaneError('kernel_connection_lost'); }
    return { ActivePromptSteered: { prompt: { id: 'taken' } } };
  };
  await assert.rejects(coordinate(operation, ['lane', review], r.options), { code: 'kernel_connection_lost' });
  // Another launch can refresh the round receipt while the accepted request's result is still unknown.
  const record = JSON.parse(await readFile(join(r.home, 'lane.json'), 'utf8'));
  record.launch.attachmentId = 'another-attachment'; await save(join(r.home, 'lane.json'), record);
  await coordinate(operation, ['lane', review], r.options);
  assert.equal(renewals, 1);
  assert.equal(commands.size, 2, 'one definitive refusal and one accepted command, replayed without a new identity');
});

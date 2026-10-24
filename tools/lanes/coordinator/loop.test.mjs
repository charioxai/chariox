// MP-08 / MP-09 / MP-10 / MP-11: each coordinator step against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { coordinate, save } from './loop.mjs';
const head = 'a'.repeat(40);
async function rig(t) {
  const home = await mkdtemp(join(tmpdir(), 'coordinator-')); t.after(() => rm(home, { recursive: true, force: true }));
  const calls = [];
  const config = { kernel: 'executor', providers: { codex: { profile: 'linked', model: 'gpt-6-luna', effort: 'low', permission: 'full' } }, repository: home, bases: { lane: head } };
  const launch = { kernel: 'executor', taskId: 'lane', sessionId: 'room', operationsSessionId: 'ops', agentId: 'agent', attachmentId: 'attached', promptId: 'prompt' };
  const options = { home, config, now: () => 1000, stdin: async () => 'review the exact PR', adapter: async (op, input) => {
    calls.push({ op, input });
    if (op === 'launch') return launch;
    if (op === 'status') return { state: 'idle', currentPrompt: null, lastActivityMs: 1, deadline: { mode: 'kernel', state: 'armed' } };
    if (op === 'fetch') { await mkdir(input.directory, { recursive: true }); await save(join(input.directory, 'ACCEPTANCE_UPDATE.json'), { rows: [{ head }] }); return { fetched: true }; }
    return { stopped: true };
  }, cli: async argv => {
    calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room' }] } };
    if (argv[1] === 'wait') return { events: [{ cursor: 1, type: 'agent' }], next_cursor: 1 };
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
  assert.deepEqual(r.calls.map(c => c.op), ['stop']);
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
test('deliver reuses the same command id for the same review and current prompt', async t => {
  const r = await launched(t); const review = join(r.home, 'review.md'); await writeFile(review, 'review');
  const adapter = r.options.adapter;
  r.options.adapter = (op, input) => op === 'status' ? { currentPrompt: 'current' } : adapter(op, input);
  await coordinate('deliver', ['lane', review], r.options); await coordinate('deliver', ['lane', review], r.options);
  assert.equal(r.calls[0].input.steerId, r.calls[1].input.steerId);
});

// MP-08 / MP-09 / MP-10 / MP-11: the standing round service against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { join } from 'node:path';
import { parseEntry, serve } from './loopd.mjs';

const sha = 'a'.repeat(40);
const entryFile = (lane, round, extra = {}) => ({ lane, round, provider: 'claude', model: 'claude-sonnet-5-5', effort: 'high', hardStopMinutes: 60, brief: '/briefs/BRIEF.md', worktrees: ['/work/' + lane], ...extra });

async function rig(t) {
  const root = await mkdtemp(join(tmpdir(), 'loopd-')); t.after(() => rm(root, { recursive: true, force: true }));
  const layout = { agents: join(root, 'agents'), queue: join(root, 'agents', '_loop', 'queue'), home: join(root, 'home') };
  await mkdir(layout.queue, { recursive: true });
  const clock = { ms: 1_000_000 }, calls = [], idle = new Set(), launches = new Map(), script = [], controller = new AbortController(), logs = [];
  const config = { kernel: 'executor', providers: { claude: { profile: 'p', model: 'm', effort: 'high', permission: 'required' }, codex: { profile: 'c', model: 'g', effort: 'low', permission: 'yolo' } } };
  const brief = join(root, 'BRIEF.md'); await writeFile(brief, 'the brief');
  const drop = async (lane, round, extra) => writeFile(join(layout.queue, `${lane}-${round}.json`), JSON.stringify(entryFile(lane, round, { brief, ...extra })));
  const ev = (key, extra) => ({ cursor: calls.length, kernel: 'executor', session_id: `room-${key}`, agent_id: `agent-${key}`, ...extra });
  const options = {
    layout, config, signal: controller.signal, now: () => clock.ms, wakeMinutes: 0.25, idleMs: 5000, retryMs: 30_000,
    log: (kind, detail) => logs.push({ kind, ...detail }),
    sleep: async ms => { clock.ms += ms; },
    git: async argv => { calls.push({ git: argv }); return sha + '\n'; },
    adapter: async (op, input) => {
      calls.push({ op, taskId: input.taskId, input });
      if (op === 'launch') { launches.set(input.taskId, true); await writeFile(input.stateFile, JSON.stringify({ promptId: `prompt-${input.taskId}` })); return { kernel: 'executor', taskId: input.taskId, sessionId: `room-${input.taskId}`, operationsSessionId: `ops-${input.taskId}`, agentId: `agent-${input.taskId}`, attachmentId: `att-${input.taskId}` }; }
      if (op === 'status') return { state: idle.has(input.taskId) ? 'idle' : 'working', currentPrompt: idle.has(input.taskId) ? null : 'prompt', lastActivityMs: clock.ms, deadline: { mode: 'kernel', state: 'armed' } };
      if (op === 'fetch') return { 'ACCEPTANCE_UPDATE.json': { missing: true, reason: 'file_not_found' } };
      if (op === 'bundle') return { base: sha, head: 'b'.repeat(40), sha256: 'c'.repeat(64), path: `/bundles/${input.taskId}.bundle` };
      if (op === 'message') return { text: `final message of ${input.taskId}` };
      return { stopped: true };
    },
    cli: async argv => {
      calls.push({ argv });
      if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [...launches.keys()].flatMap(key => [{ kernel: 'executor', session_id: `room-${key}` }, { kernel: 'executor', session_id: `room-${key}`, agent_id: `agent-${key}` }, { kernel: 'executor', session_id: `ops-${key}` }]) } };
      if (argv[1] === 'wait') {
        clock.ms += 15_000;
        const stops = calls.filter(c => c.op === 'stop' && !c.served);
        if (stops.length) { stops[0].served = true; return { events: [ev(stops[0].taskId, { type: 'session', value: { state: 'agent_removed' } })], next_cursor: calls.length }; }
        const step = script.shift();
        const events = step ? await step() : [];
        return { events: events || [], next_cursor: calls.length, timed_out: !events?.length };
      }
      return {};
    },
    respond: async () => ({}),
  };
  const finished = count => { const watcher = options.log; options.log = (kind, detail) => { watcher(kind, detail); if (logs.filter(l => l.kind === 'finished').length >= count) controller.abort(); }; };
  return { root, layout, clock, calls, idle, script, controller, options, logs, drop, ev, finished, launches };
}
const runFile = (r, lane) => readFile(join(r.layout.agents, lane, 'LOOP_RUN.json'), 'utf8').then(JSON.parse);

test('parseEntry accepts a complete entry and normalizes worktrees', () => {
  const entry = parseEntry('integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { worktrees: ['/a', { path: '/b', base: sha }], notes: '/n.md' })));
  assert.equal(entry.key, 'integ2-r35');
  assert.deepEqual(entry.worktrees, [{ path: '/a', base: undefined }, { path: '/b', base: sha }]);
  assert.equal(entry.notes, '/n.md');
  assert.deepEqual(entry.preamble, []);
  assert.deepEqual(parseEntry('integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { preamble: ['/common.md'] }))).preamble, ['/common.md']);
});

test('parseEntry names the exact field it rejects', () => {
  const bad = [
    ['integ2-r35.json', '{nope', 'file'],
    ['wrong-name.json', JSON.stringify(entryFile('integ2', 'r35')), 'file'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { provider: 'gemini' })), 'provider'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { effort: 'extreme' })), 'effort'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { hardStopMinutes: 0 })), 'hardStopMinutes'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { brief: 'relative.md' })), 'brief'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { worktrees: [] })), 'worktrees'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { worktrees: ['/a', { path: '/b', base: 'abc' }] })), 'worktrees[1].base'],
    ['../x-r1.json', JSON.stringify(entryFile('../x', 'r1')), 'lane'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { preamble: 'common.md' })), 'preamble'],
    ['integ2-r35.json', JSON.stringify(entryFile('integ2', 'r35', { preamble: ['/ok.md', 'relative.md'] })), 'preamble[1]'],
  ];
  for (const [name, text, field] of bad) assert.throws(() => parseEntry(name, text), error => error.code === 'invalid_queue_entry' && error.message.startsWith(field), `${name} ${field}`);
});

test('serve launches entries as they appear; one finishes, one stops at its hard stop', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1', { hardStopMinutes: 60 });
  r.script.push(async () => { await r.drop('beta', 'r2', { hardStopMinutes: 2 }); return []; });
  r.script.push(async () => []);
  r.script.push(async () => { r.idle.add('alpha-r1'); return [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]; });
  r.finished(2);
  await serve(r.options);
  const alpha = await runFile(r, 'alpha'), beta = await runFile(r, 'beta');
  assert.equal(alpha.finishReason, 'finished', JSON.stringify(alpha.collectError));
  assert.equal(alpha.round, 'r1');
  assert.equal(alpha.head, 'b'.repeat(40));
  assert.equal(beta.finishReason, 'hard_stop');
  assert.ok(beta.finishedAtMs >= beta.launchedAtMs + 2 * 60_000);
  assert.equal(r.calls.filter(c => c.op === 'launch').length, 2);
  assert.equal(await readFile(join(r.layout.agents, 'alpha', 'last-message.md'), 'utf8'), 'final message of alpha-r1');
  assert.deepEqual((await readdir(join(r.layout.queue, 'done'))).sort(), ['alpha-r1.json', 'beta-r2.json']);
  assert.deepEqual((await readdir(r.layout.queue)).filter(f => f.endsWith('.json')), []);
  assert.ok((await runFile(r, 'alpha')).launchedAtMs < beta.launchedAtMs);
  await readFile(join(r.layout.agents, 'alpha', 'LOOP_RUN.r1.json'), 'utf8');
});

test('serve passes the entry model, effort and deadline to launch and keeps lanes apart', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1', { model: 'claude-opus-5-5', effort: 'xhigh', hardStopMinutes: 90 });
  r.idle.add('alpha-r1');
  r.script.push(async () => [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]);
  r.finished(1);
  await serve(r.options);
  const launch = r.calls.find(c => c.op === 'launch').input;
  assert.equal(launch.model, 'claude-opus-5-5');
  assert.equal(launch.effort, 'xhigh');
  assert.equal(launch.taskId, 'alpha-r1');
  assert.equal(launch.deadlineMs, 1_000_000 + (90 + 15) * 60_000, 'deadline = hard stop plus margin from the first sighting');
  assert.equal(launch.remoteBrief.startsWith(join(r.layout.agents, 'alpha')), true);
});

test('serve rejects a bad entry with its reason and keeps serving the good one', async t => {
  const r = await rig(t);
  await writeFile(join(r.layout.queue, 'bad-r1.json'), JSON.stringify(entryFile('bad', 'r1', { provider: 'nope' })));
  await r.drop('alpha', 'r1');
  r.idle.add('alpha-r1');
  r.script.push(async () => [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]);
  r.finished(1);
  await serve(r.options);
  assert.match(await readFile(join(r.layout.queue, 'rejected', 'bad-r1.json.reason.txt'), 'utf8'), /^invalid_queue_entry: provider: must be claude or codex/);
  assert.equal((await runFile(r, 'alpha')).finishReason, 'finished');
});

test('a restarted service resumes the recorded lane and its receipts, and finishes it once', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  const first = await runFile(r, 'alpha');
  assert.equal(first.finishReason, undefined);
  assert.ok(first.launch.agentId);
  const launchedAt = first.launchedAtMs;
  // New service process, new abort signal, same queue and receipts.
  const again = new AbortController(); r.options.signal = again.signal; r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'finished') again.abort(); };
  r.idle.add('alpha-r1');
  r.script.push(async () => [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]);
  await serve(r.options);
  const done = await runFile(r, 'alpha');
  assert.equal(done.finishReason, 'finished');
  assert.equal(done.launchedAtMs, launchedAt);
  assert.equal(done.launch.agentId, first.launch.agentId);
  assert.equal(r.calls.filter(c => c.git?.includes('init')).length, 1, 'the base is fixed once, not re-read on resume');
  assert.equal((await readdir(join(r.layout.queue, 'done'))).length, 1);
});

test('a second entry for a running lane waits, and the second round starts clean after the first', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  await r.drop('alpha', 'r2');
  r.idle.add('alpha-r1');
  r.script.push(async () => [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]);
  r.finished(2);
  const watcher = r.options.log;
  r.options.log = (kind, detail) => { watcher(kind, detail); if (kind === 'picked_up' && detail.key === 'alpha-r2') { r.idle.add('alpha-r2'); r.script.push(async () => [r.ev('alpha-r2', { type: 'agent', value: { state: 'idle' } })]); } };
  await serve(r.options);
  const second = await runFile(r, 'alpha');
  assert.equal(second.round, 'r2');
  assert.equal(second.finishReason, 'finished');
  assert.equal((await readFile(join(r.layout.agents, 'alpha', 'LOOP_RUN.r1.json'), 'utf8').then(JSON.parse)).round, 'r1');
  assert.deepEqual(r.calls.filter(c => c.op === 'launch').map(c => c.taskId), ['alpha-r1', 'alpha-r2']);
});

test('a restart resumes the running round before it admits a queued one for the same lane, whatever the file order', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r9');
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  assert.equal((await runFile(r, 'alpha')).round, 'r9');
  // alpha-r10 sorts before alpha-r9 in the queue; a new service must still resume r9 alone.
  await r.drop('alpha', 'r10');
  const again = new AbortController(); r.options.signal = again.signal;
  const order = [];
  r.options.log = (kind, detail) => {
    r.logs.push({ kind, ...detail });
    if (kind === 'picked_up' || kind === 'finished') order.push(`${kind}:${detail.key}`);
    if (kind === 'picked_up') { r.idle.add(detail.key); r.script.push(async () => [r.ev(detail.key, { type: 'agent', value: { state: 'idle' } })]); }
    if (kind === 'finished' && detail.key === 'alpha-r10') again.abort();
  };
  await serve(r.options);
  assert.deepEqual(order, ['picked_up:alpha-r9', 'finished:alpha-r9', 'picked_up:alpha-r10', 'finished:alpha-r10']);
  assert.equal((await readFile(join(r.layout.agents, 'alpha', 'LOOP_RUN.r9.json'), 'utf8').then(JSON.parse)).finishReason, 'finished', 'the archived r9 file is its finished run');
});

test('the next round of a lane does not inherit the previous round handoff files, and a resume keeps its own', async t => {
  const r = await rig(t);
  const handoff = ['PUSH_READY.md', 'LANE_STATUS.md', 'ACCEPTANCE_UPDATE.json'];
  const laneDirectory = join(r.layout.agents, 'alpha');
  const present = () => Promise.all(handoff.map(name => readFile(join(laneDirectory, name), 'utf8').then(() => true, () => false)));
  const adapter = r.options.adapter, seen = [];
  r.options.adapter = async (op, input) => { if (op === 'fetch') seen.push([input.taskId, await present()]); return adapter(op, input); };
  await r.drop('alpha', 'r1');
  r.idle.add('alpha-r1');
  r.script.push(async () => { for (const name of handoff) await writeFile(join(laneDirectory, name), `round r1 ${name}`); return [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]; });
  r.finished(1);
  await serve(r.options);
  await r.drop('alpha', 'r2');
  // r2 starts, its worker writes a note, the service restarts, and r2 is hard-stopped without an acceptance file.
  const first = new AbortController(); r.options.signal = first.signal;
  r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'picked_up') r.script.push(async () => { await writeFile(join(laneDirectory, 'PUSH_READY.md'), 'round r2'); first.abort(); return []; }); };
  await serve(r.options);
  assert.equal(await readFile(join(laneDirectory, 'PUSH_READY.md'), 'utf8'), 'round r2');
  for (const name of handoff) assert.equal(await readFile(join(laneDirectory, name.replace(/(\.[a-z]+)$/, '.r1$1')), 'utf8'), `round r1 ${name}`);
  const again = new AbortController(); r.options.signal = again.signal;
  r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'finished') again.abort(); };
  r.script.push(async () => { r.clock.ms += 61 * 60_000; return []; });
  await serve(r.options);
  assert.equal(await readFile(join(laneDirectory, 'PUSH_READY.md'), 'utf8'), 'round r2', 'a resumed round keeps its own handoff');
  assert.deepEqual(seen.find(([task]) => task === 'alpha-r2')[1], [true, false, false], 'only r2 own file is visible to its collection');
  assert.equal((await runFile(r, 'alpha')).finishReason, 'hard_stop');
});

test('the preamble files are appended to the brief once, so a resume sees the same brief', async t => {
  const r = await rig(t);
  const common = join(r.root, 'common.md'); await writeFile(common, 'shared rules v1');
  await r.drop('alpha', 'r1', { preamble: [common] });
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  const launched = await readFile(join(r.layout.home, 'alpha-r1.brief.md'), 'utf8');
  assert.equal(launched, 'the brief\n\nshared rules v1\n');
  // The shared file changes while loopd is down: the running round keeps the brief it was launched with.
  await writeFile(common, 'shared rules v2');
  const again = new AbortController(); r.options.signal = again.signal; r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'finished') again.abort(); };
  r.idle.add('alpha-r1');
  r.script.push(async () => [r.ev('alpha-r1', { type: 'agent', value: { state: 'idle' } })]);
  await serve(r.options);
  assert.equal((await runFile(r, 'alpha')).finishReason, 'finished');
  assert.equal(await readFile(join(r.layout.home, 'alpha-r1.brief.md'), 'utf8'), launched);
});

test('a missing preamble file ends the entry as launch_failed with the file named', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1', { preamble: [join(r.root, 'nope.md')] });
  r.options.retryMs = 1;
  r.finished(1);
  const watcher = r.options.log; r.options.log = (kind, detail) => { watcher(kind, detail); if (kind === 'launch_failed') r.controller.abort(); };
  await serve(r.options);
  const run = await runFile(r, 'alpha');
  assert.equal(run.finishReason, 'launch_failed');
  assert.equal(run.launchError.code, 'preamble_unavailable');
  assert.match(run.launchError.message, /nope\.md/);
});

test('a restored round whose resume fails keeps its lane: the queued round waits through the retry', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r9');
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  await r.drop('alpha', 'r10');
  const again = new AbortController(); r.options.signal = again.signal;
  const adapter = r.options.adapter;
  let failures = 1;
  r.options.adapter = async (op, input) => { if (op === 'launch' && input.taskId === 'alpha-r9' && failures-- > 0) throw Object.assign(new Error('kernel down'), { code: 'kernel_unreachable' }); return adapter(op, input); };
  const order = [];
  r.options.log = (kind, detail) => {
    r.logs.push({ kind, ...detail });
    if (['picked_up', 'finished', 'launch_error'].includes(kind)) order.push(`${kind}:${detail.key}`);
    if (kind === 'picked_up') { r.idle.add(detail.key); r.script.push(async () => [r.ev(detail.key, { type: 'agent', value: { state: 'idle' } })]); }
    if (kind === 'finished' && detail.key === 'alpha-r10') again.abort();
  };
  await serve(r.options);
  assert.deepEqual(order, ['launch_error:alpha-r9', 'picked_up:alpha-r9', 'finished:alpha-r9', 'picked_up:alpha-r10', 'finished:alpha-r10']);
});

test('a restored round keeps its run, queue entry and lane after every retry fails; the queued round starts only after it ends', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r9');
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  await r.drop('alpha', 'r10');
  const again = new AbortController(); r.options.signal = again.signal;
  const adapter = r.options.adapter;
  let outage = true;
  r.options.adapter = async (op, input) => { if (op === 'launch' && input.taskId === 'alpha-r9' && outage) throw Object.assign(new Error('kernel down'), { code: 'kernel_unreachable' }); return adapter(op, input); };
  const order = [];
  let failures = 0;
  r.options.log = (kind, detail) => {
    r.logs.push({ kind, ...detail });
    if (['picked_up', 'finished', 'launch_failed'].includes(kind)) order.push(`${kind}:${detail.key}`);
    // The outage outlasts the five retries and ends a few attempts later.
    if (kind === 'launch_error' && ++failures === 8) outage = false;
    if (kind === 'picked_up') { r.idle.add(detail.key); r.script.push(async () => [r.ev(detail.key, { type: 'agent', value: { state: 'idle' } })]); }
    if (kind === 'finished' && detail.key === 'alpha-r10') again.abort();
  };
  r.options.sleep = async ms => {
    r.clock.ms += ms;
    if (failures === 6 && outage) {
      const run = await runFile(r, 'alpha');
      assert.equal(run.round, 'r9'); assert.equal(run.finishReason, undefined, 'the running round is not ended');
      assert.equal(run.driverError.code, 'kernel_unreachable', 'the lane shows the loop as down, with the reason');
      assert.deepEqual((await readdir(join(r.layout.queue, 'done'))), [], 'no entry leaves the queue');
    }
  };
  await serve(r.options);
  assert.deepEqual(order, ['picked_up:alpha-r9', 'finished:alpha-r9', 'picked_up:alpha-r10', 'finished:alpha-r10']);
  assert.equal((await readFile(join(r.layout.agents, 'alpha', 'LOOP_RUN.r9.json'), 'utf8').then(JSON.parse)).finishReason, 'finished');
});

test('the lane file records the health signals of the last status read, so a worker with no activity shows as SILENT', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'status' ? { state: 'working', currentPrompt: 'prompt', lastActivityMs: r.clock.ms - 20 * 60_000, deadline: { mode: 'kernel', state: 'armed' } } : adapter(op, input);
  for (let i = 0; i < 6; i++) r.script.push(async () => []);
  r.script.push(async () => { r.controller.abort(); return []; });
  await serve(r.options);
  const run = await runFile(r, 'alpha');
  assert.deepEqual(run.health.flags, ['SILENT']);
  assert.equal(run.health.state, 'working');
  assert.ok(run.health.atMs > run.launchedAtMs);
});

test('an entry that cannot launch is retried, then ends as launch_failed with its reason', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => { if (op === 'launch') throw Object.assign(new Error('kernel down'), { code: 'kernel_connection_lost' }); return adapter(op, input); };
  r.finished(0);
  r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'launch_failed') r.controller.abort(); };
  await serve(r.options);
  const run = await runFile(r, 'alpha');
  assert.equal(run.finishReason, 'launch_failed');
  assert.equal(run.launchError.code, 'kernel_connection_lost');
  assert.equal(r.logs.filter(l => l.kind === 'launch_error').length, 5);
  assert.equal((await readdir(join(r.layout.queue, 'done'))).length, 1);
});

test('a starting service drops the targets of workers an earlier service stopped, before any lane runs', async t => {
  const r = await rig(t);
  await mkdir(r.layout.home, { recursive: true });
  await writeFile(join(r.layout.home, 'old-r1.json'), JSON.stringify({ lane: 'old-r1', launch: { kernel: 'executor', sessionId: 'room-old', operationsSessionId: 'ops-old', agentId: 'agent-old' }, stopped: true, stop: { stopped: true } }));
  const cli = r.options.cli;
  r.options.cli = async argv => argv[1] === 'subscription'
    ? { subscription: { subscription_id: 'grant', targets: [{ kernel: 'executor', session_id: 'room-old', agent_id: 'agent-old' }], kernels: [{ kernel: 'executor' }] } }
    : cli(argv);
  r.controller.abort();
  await serve(r.options);
  assert.deepEqual(r.calls.filter(c => c.argv?.[1] === 'unsubscribe').map(c => c.argv.slice(4).join(' ')), ['--session room-old --agent agent-old']);
});

test('after a grant change the stale inbox is set aside and every running lane is re-read', async t => {
  const r = await rig(t);
  await mkdir(r.layout.home, { recursive: true });
  await writeFile(join(r.layout.home, 'subscription.json'), JSON.stringify({ subscription_id: 'old-grant' }));
  await r.drop('alpha', 'r1');
  r.idle.add('alpha-r1');
  r.finished(1);
  await serve(r.options);
  assert.equal((await runFile(r, 'alpha')).finishReason, 'finished', 'a re-read finds the finished worker without waiting for an event');
  assert.ok((await readdir(r.layout.home)).some(f => /^subscription\.\d+\.json$/.test(f)));
  assert.ok(r.logs.some(l => l.kind === 'inbox_reset'));
});

test('a service started while the kernel is unreachable keeps running and serves once it answers', async t => {
  const r = await rig(t);
  await mkdir(r.layout.home, { recursive: true });
  await writeFile(join(r.layout.home, 'subscription.json'), JSON.stringify({ subscription_id: 'grant' }));
  await r.drop('alpha', 'r1');
  r.idle.add('alpha-r1');
  r.finished(1);
  const cli = r.options.cli;
  let down = 2;
  r.options.cli = async argv => { if (argv[1] === 'subscription' && down-- > 0) throw Object.assign(new Error('kernel down'), { code: 'kernel_unreachable' }); return cli(argv); };
  await serve(r.options);
  assert.equal((await runFile(r, 'alpha')).finishReason, 'finished');
  assert.ok(r.logs.some(l => l.kind === 'inbox_unavailable' && l.code === 'kernel_unreachable'), 'the outage is logged, not fatal');
});

test('after a restart gives the service a new grant, the running lanes follow the new inbox and finish', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'picked_up') r.controller.abort(); };
  await serve(r.options);
  await writeFile(join(r.layout.home, 'subscription.json'), JSON.stringify({ subscription_id: 'old-grant' }));
  const followed = [];
  const cli = r.options.cli;
  r.options.cli = async argv => {
    if (argv[1] === 'subscribe') { followed.push(argv.slice(2).join(' ')); r.calls.push({ argv }); return {}; }
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'new-grant', targets: followed.map(item => ({ kernel: 'executor', session_id: item.split(' ')[3], ...(item.includes('--agent') && { agent_id: item.split(' ')[5] }) })), kernels: [{ kernel: 'executor' }] } };
    return cli(argv);
  };
  r.options.signal = (r.controller = new AbortController()).signal;
  r.idle.add('alpha-r1');
  r.options.log = (kind, detail) => { r.logs.push({ kind, ...detail }); if (kind === 'finished') r.controller.abort(); };
  await serve(r.options);
  assert.deepEqual(followed.sort(), ['--kernel executor --session ops-alpha-r1', '--kernel executor --session room-alpha-r1', '--kernel executor --session room-alpha-r1 --agent agent-alpha-r1']);
  assert.equal((await runFile(r, 'alpha')).finishReason, 'finished');
  assert.equal(r.calls.filter(c => c.op === 'launch').length, 2, 'the same receipts are reused; the adapter sees the idempotent launch only');
});

test('a decision raised while no inbox followed the worker is read from its status and answered once', async t => {
  const r = await rig(t);
  await r.drop('alpha', 'r1');
  const adapter = r.options.adapter;
  const answers = [];
  r.options.adapter = async (op, input) => op === 'status' && input.taskId === 'alpha-r1' && !answers.length
    ? { ...await adapter(op, input), interactions: [{ id: 'perm-gap', kind: 'permission', title: 'Approve Claude Code Write?', choices: ['allow_once'] }] }
    : adapter(op, input);
  r.options.respond = async input => { answers.push(input); r.idle.add('alpha-r1'); return {}; };
  r.finished(1);
  await serve(r.options);
  assert.deepEqual(answers.map(a => [a.interactionId, a.choiceId]), [['perm-gap', 'allow_once']]);
  assert.equal((await runFile(r, 'alpha')).permissions.length, 1);
});

test('round.mjs serve starts as a real process, serves the queue and stops cleanly on SIGTERM', async t => {
  const root = await mkdtemp(join(tmpdir(), 'loopd-process-')); t.after(() => rm(root, { recursive: true, force: true }));
  const home = join(root, 'home'), agents = join(root, 'agents');
  await mkdir(home); await mkdir(agents);
  await writeFile(join(home, 'config.json'), JSON.stringify({ providers: {} }));
  await writeFile(join(agents, 'x'), '');
  const child = spawn(process.execPath, [new URL('./round.mjs', import.meta.url).pathname, 'serve'], { env: { PATH: process.env.PATH, CHARIOX_COORDINATOR_HOME: home, LOOPD_AGENTS: agents }, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', chunk => { output += chunk; });
  const exited = once(child, 'exit');
  for (let i = 0; i < 100 && !output.includes('"started"'); i++) await new Promise(resolve => setTimeout(resolve, 100));
  assert.match(output, /"kind":"started"/);
  assert.ok((await readdir(join(agents, '_loop', 'queue'))).includes('done'), 'the queue directories exist once it serves');
  child.kill('SIGTERM');
  const [code] = await exited;
  assert.equal(code, 0);
  assert.match(output, /"kind":"stopped"/);
});

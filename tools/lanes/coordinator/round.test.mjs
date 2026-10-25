// MP-08 / MP-09 / MP-10 / MP-11: the multi-lane round against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile, mkdir, readFile, appendFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runRound, newSections } from './round.mjs';
import { save, coordinate } from './loop.mjs';
const head = 'b'.repeat(40);

async function rig(t, lanes = ['alpha', 'beta']) {
  const home = await mkdtemp(join(tmpdir(), 'round-')); t.after(() => rm(home, { recursive: true, force: true }));
  const clock = { ms: 1_000_000 }, calls = [], waits = [], idle = new Set(), launches = {}, interactions = {};
  const specs = [];
  for (const lane of lanes) {
    await writeFile(join(home, `${lane}.brief.src.md`), `brief ${lane}`);
    await writeFile(join(home, `${lane}.inbox.md`), '## 10:00Z earlier note\nalready there\n');
    launches[lane] = { kernel: 'executor', taskId: lane, sessionId: `room-${lane}`, operationsSessionId: `ops-${lane}`, agentId: `agent-${lane}`, attachmentId: `att-${lane}`, promptId: `prompt-${lane}` };
    specs.push({ lane, worktree: `/work/${lane}`, provider: 'claude', model: 'm', brief: join(home, `${lane}.brief.src.md`), inbox: join(home, `${lane}.inbox.md`), runFile: join(home, 'runs', `${lane}.json`), hardStopMinutes: 60 });
  }
  await mkdir(join(home, 'runs'));
  const ev = (lane, extra) => ({ cursor: waits.length + 1, kernel: 'executor', session_id: `room-${lane}`, agent_id: `agent-${lane}`, ...extra });
  const options = {
    home, now: () => clock.ms, stdin: async () => '',
    config: { kernel: 'executor', providers: { claude: { profile: 'p', model: 'm', effort: 'high', permission: 'required' } }, repository: home, bases: Object.fromEntries(lanes.map(l => [l, head])) },
    adapter: async (op, input) => {
      calls.push({ op, taskId: input.taskId, input });
      if (op === 'launch') return launches[input.taskId];
      if (op === 'steer') return { ActivePromptSteered: { prompt: { id: 'steer-id' } } };
      if (op === 'status') return { completedTurn: { promptId: `prompt-${input.taskId}`, status: 'completed' }, state: idle.has(input.taskId) ? 'idle' : 'working', currentPrompt: idle.has(input.taskId) ? null : 'prompt', lastActivityMs: clock.ms, interactions: interactions[input.taskId] ?? [], deadline: { mode: 'kernel', state: 'armed' } };
      if (op === 'fetch') return { 'ACCEPTANCE_UPDATE.json': { missing: true, reason: 'file_not_found' } };
      if (op === 'bundle') return { base: head, head, sha256: 'c'.repeat(64), path: `/bundles/${input.taskId}.bundle` };
      return { stopped: true };
    },
    cli: async argv => {
      calls.push({ argv });
      if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: lanes.flatMap(l => [{ kernel: 'executor', session_id: `room-${l}` }, { kernel: 'executor', session_id: `room-${l}`, agent_id: `agent-${l}` }, { kernel: 'executor', session_id: `ops-${l}` }]) } };
      if (argv[1] === 'wait') {
        const stops = calls.filter(c => c.op === 'stop' && !c.served);
        if (stops.length) stops[0].served = true;
        const next = stops.length ? { events: [ev(stops[0].taskId, { type: 'session', value: { state: 'agent_removed' } })] } : waits.shift();
        if (!next) throw new Error('script exhausted'); clock.ms += 60_000; return { events: next.events || [], next_cursor: waits.length + 1, timed_out: !next.events?.length }; }
      return {};
    },
    respond: async input => { calls.push({ op: 'respond', input }); return {}; },
  };
  return { home, clock, calls, waits, idle, interactions, options, ev, spec: { home, wakeMinutes: 5, lanes: specs } };
}

test('newSections splits appended sections and ignores earlier bytes', () => {
  const text = '## a\none\n## b\ntwo\n';
  assert.deepEqual(newSections(text, Buffer.byteLength('## a\none\n')), ['## b\ntwo\n']);
  assert.deepEqual(newSections(text, Buffer.byteLength(text)), []);
});

test('round: permission answered, note delivered once, finish collected, hard stop stops', async t => {
  const r = await rig(t);
  r.interactions.alpha = [{ id: 'claude-permission-1', kind: 'permission', title: 'Approve Claude Code Read?', choices: ['allow_once'] }];
  r.waits.push({ events: [r.ev('alpha', { at_ms: 123_456, type: 'interaction', refs: { interaction_id: 'claude-permission-1' }, summary: 'Approve Claude Code Read?', value: { state: 'pending' } }),
    r.ev('beta', { type: 'interaction', refs: { interaction_id: 'ask-1' }, summary: 'Question for the user', value: { state: 'pending' } })] });
  r.waits.push({});
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  // Beta never finishes: the third wake is past its hard stop.
  const before = r.options.cli;
  let wakes = 0;
  r.options.cli = async argv => { if (argv[1] === 'wait' && ++wakes === 2) { await appendFile(join(r.home, 'alpha.inbox.md'), '## 11:00Z coordinator\nplease change X\n'); } if (argv[1] === 'wait' && wakes === 3) { r.idle.add('alpha'); } if (argv[1] === 'wait' && wakes === 4) r.clock.ms += 3_600_000; return before(argv); };
  r.waits.push({});
  const [alpha, beta] = await runRound(r.spec, r.options);
  assert.equal(r.calls.filter(c => c.op === 'respond').length, 1);
  assert.equal(alpha.permissions[0].choice, 'allow_once');
  assert.equal(alpha.lastEventAtMs, 123_456);
  assert.deepEqual(beta.unanswered.map(u => u.interactionId), ['ask-1']);
  assert.equal(alpha.notes.length, 1);
  assert.equal(alpha.notes[0].delivered, true);
  assert.match(alpha.notes[0].heading, /11:00Z coordinator/);
  const steers = r.calls.filter(c => c.op === 'steer');
  assert.equal(steers.length, 1);
  assert.match(steers[0].input.prompt, /please change X/);
  assert.doesNotMatch(steers[0].input.prompt, /already there/);
  assert.equal(alpha.finishReason, 'finished', JSON.stringify(alpha.collectError));
  assert.equal(alpha.head, head);
  assert.equal(alpha.bundleSha256, 'c'.repeat(64));
  assert.equal(beta.finishReason, 'hard_stop');
  assert.equal(beta.stopped, true);
  assert.equal(JSON.parse(await readFile(join(r.home, 'runs', 'alpha.json'), 'utf8')).finishReason, 'finished');
  // No event ever touched beta: its status was read only by the first-minute and five-minute reconciliation, not on every wake.
  assert.ok(r.calls.filter(c => c.op === 'status' && c.taskId === 'beta').length <= 13);
});

test('round: a rerun resumes the recorded round without relaunching or redelivering', async t => {
  const r = await rig(t, ['alpha']);
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.idle.add('alpha');
  const [first] = await runRound(r.spec, r.options);
  assert.equal(first.finishReason, 'finished');
  const launches = r.calls.filter(c => c.op === 'launch').length;
  const [again] = await runRound(r.spec, r.options);
  assert.equal(again.finishReason, 'finished');
  assert.equal(r.calls.filter(c => c.op === 'launch').length, launches);
});

test('lanefin after stop collects without a status read of the removed worker', async t => {
  const r = await rig(t, ['alpha']);
  const { coordinate } = await import('./loop.mjs');
  await coordinate('brief', ['alpha', '/work/alpha', 'claude', r.spec.lanes[0].brief], r.options);
  await coordinate('stop', ['alpha'], r.options);
  r.calls.length = 0;
  const result = await coordinate('lanefin', ['alpha'], r.options);
  assert.equal(result.bundle.head, head);
  assert.equal(r.calls.some(c => c.op === 'status'), false);
});

test('round: a free-form acceptance head still finishes with head, bundle and the field to fix', async t => {
  const r = await rig(t, ['alpha']);
  const adapter = r.options.adapter, issues = [{ field: 'rows[0].head', problem: 'must be one full 40-character git commit id' }];
  r.options.adapter = async (op, input) => op === 'fetch' ? { 'ACCEPTANCE_UPDATE.json': { path: 'x', sha256: 'y', acceptance: { valid: false, issues } } } : adapter(op, input);
  r.idle.add('alpha');
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'finished', JSON.stringify(alpha.collectError));
  assert.equal(alpha.head, head);
  assert.equal(alpha.bundleSha256, 'c'.repeat(64));
  assert.deepEqual(alpha.acceptanceIssues, issues);
});

test('round: a finish seen in the first minute is read at the next wake, and the final message is saved before stop', async t => {
  const r = await rig(t, ['alpha']);
  const file = join(r.home, 'alpha.last-message.md');
  r.spec.lanes[0].lastMessageFile = file;
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'message' ? (r.calls.push({ op }), { text: 'all done' }) : adapter(op, input);
  r.idle.add('alpha');
  r.options.now = () => r.clock.ms;
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.clock.ms = 1_000_000; // launch time; the first event arrives 15 s later, before the status read is allowed
  const cli = r.options.cli;
  r.options.cli = async argv => { const out = await cli(argv); if (argv[1] === 'wait') r.clock.ms -= 45_000; return out; };
  r.waits.push({}); r.waits.push({}); r.waits.push({});
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'finished');
  assert.equal(await readFile(file, 'utf8'), 'all done');
  assert.deepEqual(alpha.lastMessage, { path: file, chars: 8, truncated: false });
  assert.ok(r.calls.findIndex(c => c.op === 'message') < r.calls.findIndex(c => c.op === 'stop'));
});

test('round: a worker that finished without any event is found by the first-minute read', async t => {
  const r = await rig(t, ['alpha']);
  r.idle.add('alpha');
  for (let i = 0; i < 3; i++) r.waits.push({});
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'finished');
  assert.equal(alpha.statusReads, 1);
});

test('round: a worker that goes idle without an event is found by the five-minute reconciliation', async t => {
  const r = await rig(t, ['alpha']);
  for (let i = 0; i < 12; i++) r.waits.push({});
  const cli = r.options.cli;
  r.options.cli = async argv => { if (argv[1] === 'wait' && r.clock.ms >= 1_000_000 + 4 * 60_000) r.idle.add('alpha'); return cli(argv); };
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'finished');
  assert.ok(alpha.finishedAtMs >= alpha.launchedAtMs + 5 * 60_000 && alpha.finishedAtMs <= alpha.launchedAtMs + 8 * 60_000, `${alpha.finishedAtMs - alpha.launchedAtMs}`);
});

test('round: a hard stop whose removal event never arrives still collects the head and bundle', async t => {
  const r = await rig(t, ['alpha']);
  const { coordinate } = await import('./loop.mjs');
  const cli = r.options.cli;
  r.options.cli = async argv => argv[1] === 'wait' && r.calls.some(c => c.op === 'stop') ? { events: [], timed_out: true, next_cursor: 0 } : cli(argv);
  r.spec.lanes[0].hardStopMinutes = 1;
  for (let i = 0; i < 3; i++) r.waits.push({});
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'hard_stop');
  assert.equal(alpha.head, head);
  assert.equal(alpha.bundleSha256, 'c'.repeat(64));
  assert.equal(alpha.stopped, true);
  assert.equal(alpha.stopConfirmationError.code, 'stop_confirmation_timeout');
  assert.equal(JSON.parse(await readFile(join(r.home, 'alpha.json'), 'utf8')).stopped, true);
});

test('round: extra repositories get their own verified bundle', async t => {
  const r = await rig(t, ['alpha']);
  r.spec.lanes[0].extraBundles = [{ name: 'cloud', worktree: '/work/alpha-cloud', base: 'd'.repeat(40), repository: '/repo/cloud.git' }, { name: 'bad', worktree: '/x', base: 'nope', repository: '/r' }];
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.idle.add('alpha');
  const [run] = await runRound(r.spec, r.options);
  const bundle = r.calls.filter(c => c.op === 'bundle' && c.input.taskId === 'alpha-cloud');
  assert.equal(bundle.length, 1);
  assert.equal(bundle[0].input.worktree, '/work/alpha-cloud');
  assert.match(bundle[0].input.remoteBundle, /cloud\.bundle$/);
  assert.equal(run.extraBundles[0].sha256, 'c'.repeat(64));
  assert.deepEqual(run.extraBundles[1], { name: 'bad', error: 'invalid_base' });
  assert.equal(run.finishReason, 'finished');
});

test('a lane profile overrides the provider model for that lane only', async t => {
  const r = await rig(t);
  const { coordinate } = await import('./loop.mjs');
  r.options.config.deadlineSeconds = 3600; r.options.config.laneProfiles = { alpha: { model: 'claude-opus-5-5', deadlineSeconds: 1800 } };
  for (const spec of r.spec.lanes) await coordinate('brief', [spec.lane, spec.worktree, 'claude', spec.brief], r.options);
  const models = Object.fromEntries(r.calls.filter(c => c.op === 'launch').map(c => [c.taskId, c.input.model]));
  assert.deepEqual(models, { alpha: 'claude-opus-5-5', beta: 'm' });
  const deadlines = Object.fromEntries(r.calls.filter(c => c.op === 'launch').map(c => [c.taskId, c.input.deadlineMs]));
  assert.equal(deadlines.beta - deadlines.alpha, 3_600_000 - 1_800_000);
});

test('round: a failing inbox records why the driver stopped and a rerun resumes', async t => {
  const r = await rig(t, ['alpha']);
  const cli = r.options.cli;
  r.options.cli = async argv => { if (argv[1] === 'wait') throw Object.assign(new Error('inbox gap'), { code: 'event_stream_incomplete' }); return cli(argv); };
  await assert.rejects(runRound(r.spec, r.options), { code: 'event_stream_incomplete' });
  const failed = JSON.parse(await readFile(join(r.home, 'runs', 'alpha.json'), 'utf8'));
  assert.equal(failed.driverError.code, 'event_stream_incomplete');
  r.options.cli = cli;
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.idle.add('alpha');
  const [run] = await runRound(r.spec, r.options);
  assert.equal(run.finishReason, 'finished');
  assert.equal(run.driverError, undefined);
  assert.equal(run.previousDriverErrors[0].code, 'event_stream_incomplete');
});

test('round: a note that could not be delivered stays pending, is retried with the same content, and survives a driver restart', async t => {
  const r = await rig(t, ['alpha']);
  const adapter = r.options.adapter;
  let failures = 2;
  const prompts = [];
  r.options.adapter = async (op, input) => { if (op === 'steer') prompts.push(input.prompt); if (op === 'steer' && failures-- > 0) throw Object.assign(new Error('timeout'), { code: 'kernel_request_timeout' }); return adapter(op, input); };
  const cli = r.options.cli;
  let wakes = 0;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && ++wakes === 1) await appendFile(join(r.home, 'alpha.inbox.md'), '## 11:00Z coordinator\nplease change X\n');
    if (argv[1] === 'wait' && wakes === 3) throw Object.assign(new Error('driver stopped'), { code: 'event_stream_incomplete' });
    return cli(argv);
  };
  r.waits.push({}, {}, {});
  await assert.rejects(runRound(r.spec, r.options), { code: 'event_stream_incomplete' });
  const pending = JSON.parse(await readFile(join(r.home, 'runs', 'alpha.json'), 'utf8'));
  assert.equal(pending.notes.length, 0, 'nothing is recorded as delivered or skipped');
  assert.equal(pending.noteFailures.count, 2);
  let resumed = 0;
  r.options.cli = async argv => { if (argv[1] === 'wait' && ++resumed === 2) r.idle.add('alpha'); return cli(argv); };
  r.waits.push({}, { events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const [run] = await runRound(r.spec, r.options);
  assert.deepEqual(run.notes.map(n => [n.delivered, n.reason]), [[true, undefined]]);
  assert.equal(run.noteFailures, undefined);
  assert.equal(prompts.length, 3);
  assert.ok(prompts.every(prompt => prompt === prompts[0]) && /please change X/.test(prompts[0]));
});

test('round: a permission reply that failed is retried from the lane status until it succeeds', async t => {
  const r = await rig(t, ['alpha']);
  const respond = r.options.respond;
  let failures = 1;
  r.options.respond = async input => { if (failures-- > 0) throw Object.assign(new Error('link down'), { code: 'kernel_connection_lost' }); return respond(input); };
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => {
    const status = await adapter(op, input);
    return op === 'status' && !r.calls.some(c => c.op === 'respond') ? { ...status, interactions: [{ id: 'claude-permission-1', kind: 'permission', title: 'Approve Claude Code Read?', choices: ['allow_once'] }] } : status;
  };
  r.waits.push({ events: [r.ev('alpha', { type: 'interaction', refs: { interaction_id: 'claude-permission-1' }, summary: 'Approve Claude Code Read?', value: { state: 'pending' } })] });
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.idle.add('alpha');
  const [run] = await runRound(r.spec, r.options);
  assert.equal(r.calls.filter(c => c.op === 'respond').length, 1);
  assert.deepEqual(run.permissions.map(p => [p.interactionId, p.choice, p.attempts]), [['claude-permission-1', 'allow_once', 2]]);
  assert.equal(run.permissions[0].error, undefined);
});

test('round: a completion wake received while another lane is stopping is not lost', async t => {
  const r = await rig(t);
  r.spec.lanes[0].hardStopMinutes = 2;
  const cli = r.options.cli;
  let combined = false;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && !combined && r.calls.some(c => c.op === 'stop')) {
      combined = true;
      r.calls.find(c => c.op === 'stop').served = true;
      r.idle.add('beta');
      r.clock.ms += 15_000;
      return { events: [r.ev('alpha', { cursor: 100, type: 'session', value: { state: 'agent_removed' } }), r.ev('beta', { cursor: 101, type: 'agent', value: { state: 'idle' } })], next_cursor: 101 };
    }
    return cli(argv);
  };
  r.waits.push({}, {});
  const [alpha, beta] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'hard_stop');
  assert.equal(beta.finishReason, 'finished', 'beta is read in the same wake, not at its hard stop');
});

test('round: events consumed while another lane stops are handled even when its removal never arrives', async t => {
  const r = await rig(t);
  r.spec.lanes[0].hardStopMinutes = 2;
  const cli = r.options.cli;
  let combined = false;
  r.interactions.beta = [{ id: 'claude-permission-9', kind: 'permission', title: 'Approve Claude Code Read?', choices: ['allow_once'] }];
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && r.calls.some(c => c.op === 'stop')) {
      if (combined) { r.clock.ms += 60_000; return { events: [], timed_out: true, next_cursor: 0 }; }
      combined = true;
      r.idle.add('beta');
      r.clock.ms += 15_000;
      return { events: [
        r.ev('beta', { cursor: 100, type: 'interaction', refs: { interaction_id: 'claude-permission-9' }, summary: 'Approve Claude Code Read?', value: { state: 'pending' } }),
        r.ev('beta', { cursor: 101, type: 'agent', value: { state: 'idle' } })], next_cursor: 101 };
    }
    return cli(argv);
  };
  r.waits.push({}, {});
  const [alpha, beta] = await runRound(r.spec, r.options);
  assert.equal(alpha.stopConfirmationError.code, 'stop_confirmation_timeout');
  assert.equal(r.calls.filter(c => c.op === 'respond' && c.input.interactionId === 'claude-permission-9').length, 1, 'the permission consumed during the stop is answered');
  assert.equal(beta.finishReason, 'finished', 'beta is read at once, not at its hard stop');
});

test('round: a routine permission is answered for the session when the kernel offers it, else once', async t => {
  const r = await rig(t, ['alpha']);
  const adapter = r.options.adapter;
  const offered = { 'claude-permission-1': ['allow_once', 'allow_session', 'deny'], 'claude-permission-2': ['allow_once', 'deny'] };
  r.options.adapter = async (op, input) => {
    const status = await adapter(op, input);
    return op === 'status' ? { ...status, interactions: Object.entries(offered).map(([id, choices]) => ({ id, kind: 'permission', title: 'Approve Claude Code Read?', choices })) } : status;
  };
  const pending = id => r.ev('alpha', { type: 'interaction', refs: { interaction_id: id }, summary: 'Approve Claude Code Read?', value: { state: 'pending' } });
  r.waits.push({ events: [pending('claude-permission-1'), pending('claude-permission-2')] });
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.idle.add('alpha');
  const [run] = await runRound(r.spec, r.options);
  assert.deepEqual(r.calls.filter(c => c.op === 'respond').map(c => [c.input.interactionId, c.input.choiceId]), [['claude-permission-1', 'allow_session'], ['claude-permission-2', 'allow_once']]);
  assert.deepEqual(run.permissions.map(p => p.choice), ['allow_session', 'allow_once']);
  assert.deepEqual(run.permissions.map(p => p.offered), [['allow_once', 'allow_session', 'deny'], ['allow_once', 'deny']], 'the run records what the kernel offered');
});

test('round: critical, credential and unreadable decisions are left for the coordinator, not answered', async t => {
  const r = await rig(t, ['alpha']);
  const permission = (id, extra) => ({ id, kind: 'permission', level: 'warning', title: 'Approve Claude Code Bash?', choices: ['allow_once', 'allow_session'], ...extra });
  r.interactions.alpha = [permission('crit', { level: 'critical' }), permission('cred', { title: 'Approve Claude Code passkey entry?' }), permission('ok')];
  const pending = id => r.ev('alpha', { type: 'interaction', refs: { interaction_id: id }, summary: 'Approve Claude Code Bash?', value: { state: 'pending' } });
  r.waits.push({ events: [pending('crit'), pending('cred'), pending('ok'), pending('missing')] });
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const before = r.options.cli;
  r.options.cli = async argv => { if (argv[1] === 'wait' && r.waits.length === 1) r.idle.add('alpha'); return before(argv); };
  const [alpha] = await runRound(r.spec, r.options);
  assert.deepEqual(r.calls.filter(c => c.op === 'respond').map(c => c.input.interactionId), ['ok'], 'only the routine one is answered');
  assert.deepEqual(alpha.unanswered.map(u => [u.interactionId, u.reason]), [['crit', 'critical'], ['cred', 'credential_related'], ['missing', 'unclassified']]);
});

test('round: a note the coordinator already steered directly is recorded, not delivered again; another one still is', async t => {
  const r = await rig(t, ['alpha']);
  const direct = '## 11:00Z coordinator\nsteered by the coordinator\n';
  const fallback = '## 11:05Z coordinator\nonly appended\n';
  await writeFile(join(r.home, 'alpha.direct.json'), JSON.stringify({ sections: [{ sha256: createHash('sha256').update(direct.trim()).digest('hex'), mode: 'steered', at_ms: 1 }] }));
  r.waits.push({});
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const before = r.options.cli; let wakes = 0;
  r.options.cli = async argv => { if (argv[1] === 'wait' && ++wakes === 1) await appendFile(join(r.home, 'alpha.inbox.md'), direct + fallback); if (argv[1] === 'wait' && wakes === 2) r.idle.add('alpha'); return before(argv); };
  const [alpha] = await runRound(r.spec, r.options);
  assert.deepEqual(alpha.notes.map(n => [n.mode, n.delivered]), [['direct-steered', true], ['acknowledged-steer-status', true]]);
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1, 'only the section that was not steered directly goes out');
  assert.match(r.calls.find(c => c.op === 'steer').input.prompt, /only appended/);
});

test('round: a note with several headings steered directly is not delivered again section by section from the inbox', async t => {
  const r = await rig(t, ['alpha']);
  const note = '## 11:00Z coordinator\nfirst part\n## 11:01Z coordinator\nsecond part\n';
  const file = join(r.home, 'note.src.md'); await writeFile(file, note);
  r.waits.push({});
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const before = r.options.cli; let wakes = 0;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && ++wakes === 1) { await coordinate('steer', ['alpha', file], r.options); await appendFile(join(r.home, 'alpha.inbox.md'), note); }
    if (argv[1] === 'wait' && wakes === 2) r.idle.add('alpha');
    return before(argv);
  };
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1, 'only the direct steer reached the worker');
  assert.deepEqual(alpha.notes.map(n => [n.mode, n.delivered]), [['direct-steered', true], ['direct-steered', true]]);
});

test('round: a prompt that reaches the worker after it was read idle defers the collection instead of stopping it', async t => {
  const r = await rig(t, ['alpha']);
  const base = r.options.adapter; let armed = false, reads = 0;
  r.options.adapter = async (op, input) => {
    const answer = await base(op, input);
    if (op !== 'status' || !armed) return answer;
    // read 1 is the engine's idle read; read 2 (the lanefin check on the old code, the re-read under the lock on the new) finds the prompt admitted since
    return ++reads === 2 ? { ...answer, state: 'working', currentPrompt: 'prompt-from-coordinator' } : answer;
  };
  r.waits.push({});
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  r.waits.push({});
  r.waits.push({});
  const before = r.options.cli; let wakes = 0;
  r.options.cli = async argv => { if (argv[1] === 'wait' && ++wakes === 2) { r.idle.add('alpha'); armed = true; } return before(argv); };
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(alpha.finishReason, 'finished', 'the new turn was monitored and the round ended normally');
  assert.equal(r.calls.filter(c => c.op === 'stop').length, 1, 'the worker was stopped once, after the new turn ended');
  assert.ok(reads >= 3, 'the worker was read again after the deferral');
});

for (const format of ['legacy', 'by-identity']) test(`round: a ${format} lost direct note is replayed from the inbox under its saved command id`, async t => {
  const r = await rig(t, ['alpha']);
  const note = '## 11:00Z coordinator\nplease also do Y\n';
  const digest = createHash('sha256').update(note.trim()).digest('hex');
  const pending = { sha256: digest, operation: 'prompt', id: `direct-${digest}`, parts: [digest], prompt: note };
  const ledger = { sections: [{ sha256: 'another-note', mode: 'steered' }], ...(format === 'legacy' ? { pending } : { pendingDeliveries: { [digest]: pending } }) };
  await writeFile(join(r.home, 'alpha.direct.json'), JSON.stringify(ledger));
  r.waits.push({});
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const before = r.options.cli; let wakes = 0;
  r.options.cli = async argv => { if (argv[1] === 'wait' && ++wakes === 1) await appendFile(join(r.home, 'alpha.inbox.md'), note); if (argv[1] === 'wait' && wakes === 2) r.idle.add('alpha'); return before(argv); };
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 0, 'no fresh steer is built from the worker current turn');
  const prompts = r.calls.filter(c => c.op === 'prompt');
  assert.equal(prompts.length, 1);
  assert.equal(prompts[0].input.commandId, `direct-${digest}`);
  assert.deepEqual(alpha.notes.map(n => n.delivered), [true]);
});

test('round: an invalid ACCEPTANCE_UPDATE.json is steered while the worker runs, and a worker that ends its turn with one gets the problems as its next prompt', async t => {
  const r = await rig(t, ['alpha']);
  const runs = join(r.home, 'runs');
  const bad = JSON.stringify({ rows: [{ id: 'area/ROW-1', status: 'PASS', head: 'abc1234', date: '2026-10-25' }] });
  await writeFile(join(runs, 'ACCEPTANCE_UPDATE.json'), bad);
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', summary: 'working', value: {} })] });
  r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const before = r.options.cli; let wakes = 0;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && ++wakes === 2) {
      r.idle.add('alpha'); // the worker ends its turn without fixing the file
      await writeFile(join(runs, 'ACCEPTANCE_UPDATE.json'), JSON.stringify({ rows: [{ id: 'area/ROW-1', status: 'PASS', head: 'a'.repeat(40), date: '2026-10-25' }] }));
    }
    return before(argv);
  };
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'prompt' ? (r.calls.push({ op, input }), { PromptSubmitted: { outcome: { Started: { prompt: { id: 'again' } } } } }) : adapter(op, input);
  r.waits.push({});
  const [alpha] = await runRound(r.spec, r.options);
  const steer = r.calls.find(c => c.op === 'steer');
  assert.match(steer.input.prompt, /rows\[0\]\.head: must be one full 40-character git commit id/);
  assert.match(steer.input.prompt, /check 1 of 3/);
  assert.equal(alpha.acceptanceNudges.length, 1, 'valid by the time the worker ended its turn: no second nudge, the round is collected');
  assert.equal(alpha.finishReason, 'finished');
});

test('round: a worker that ends its turn with an invalid file is prompted again, at most three times, and then collected with the problems', async t => {
  const r = await rig(t, ['alpha']);
  await writeFile(join(r.home, 'runs', 'ACCEPTANCE_UPDATE.json'), JSON.stringify({ rows: [{ id: 'bad', status: 'PASS', head: 'x', date: 'now' }] }));
  r.idle.add('alpha');
  for (let i = 0; i < 6; i++) r.waits.push({ events: [r.ev('alpha', { type: 'agent', value: { state: 'idle' } })] });
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'prompt' ? (r.calls.push({ op, input }), { PromptSubmitted: { outcome: { Started: { prompt: { id: `n${r.calls.length}` } } } } }) : adapter(op, input);
  const [alpha] = await runRound(r.spec, r.options);
  assert.equal(r.calls.filter(c => c.op === 'prompt').length, 3);
  assert.equal(alpha.acceptanceNudges.length, 3);
  assert.equal(alpha.finishReason, 'finished');
});

test('round: a quiet worker with an invalid ACCEPTANCE_UPDATE.json is steered at the next wake, not at the five-minute reconcile', async t => {
  const r = await rig(t, ['alpha']);
  const bad = JSON.stringify({ rows: [{ id: 'area/ROW-1', status: 'PASS', head: 'abc1234', date: '2026-10-25' }] });
  let wakes = 0, steeredAtWake;
  const before = r.options.cli;
  r.options.cli = async argv => {
    if (argv[1] === 'wait') { wakes++; if (r.calls.some(c => c.op === 'steer')) steeredAtWake ??= wakes; }
    if (argv[1] === 'wait' && wakes === 2) await writeFile(join(r.home, 'runs', 'ACCEPTANCE_UPDATE.json'), bad);
    return before(argv);
  };
  r.spec.lanes[0].hardStopMinutes = 6; // the round ends at its hard stop
  r.waits.push(...Array.from({ length: 10 }, () => ({})));
  await runRound(r.spec, r.options);
  assert.ok(steeredAtWake <= 4, `steered at wake ${steeredAtWake}: the file is checked on every wake, not only when the worker is read`);
});

test('a restored idle worker without a successful completed turn is never collected as finished', async t => {
  const { createRound } = await import('./engine.mjs');
  const r = await rig(t, ['alpha']);
  const first = createRound(r.options); await first.add(r.spec.lanes[0]);
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => { const value = await adapter(op, input); if (op === 'status') delete value.completedTurn; return value; };
  r.idle.add('alpha');
  const resumed = createRound(r.options); const lane = await resumed.add(r.spec.lanes[0]);
  r.waits.push({}); await resumed.step(1);
  assert.equal(lane.finished, false, 'recovery has not proved that the interrupted prompt finished');
  assert.equal(r.calls.some(c => c.op === 'stop'), false);
  r.clock.ms += 360_000;
  r.waits.push({}); await resumed.step(1);
  assert.equal(lane.run.finishReason, 'worker_lost');
});

test('a missing worker is collected as worker_lost and never as a successful round', async t => {
  const { createRound } = await import('./engine.mjs');
  const { LaneError } = await import('../lanes.mjs');
  const r = await rig(t, ['alpha']);
  const adapter = r.options.adapter;
  r.options.adapter = (op, input) => op === 'status' ? Promise.reject(new LaneError('agent_not_in_session')) : adapter(op, input);
  const engine = createRound(r.options); const lane = await engine.add(r.spec.lanes[0]);
  r.waits.push({}); await engine.step(1);
  assert.equal(lane.run.finishReason, 'worker_lost');
});

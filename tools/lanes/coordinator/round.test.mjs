// MP-08 / MP-09 / MP-10 / MP-11: the multi-lane round against fake adapters.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, mkdir, readFile, appendFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runRound, newSections } from './round.mjs';
import { save } from './loop.mjs';
const head = 'b'.repeat(40);

async function rig(t, lanes = ['alpha', 'beta']) {
  const home = await mkdtemp(join(tmpdir(), 'round-')); t.after(() => rm(home, { recursive: true, force: true }));
  const clock = { ms: 1_000_000 }, calls = [], waits = [], idle = new Set(), launches = {};
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
      if (op === 'status') return { state: idle.has(input.taskId) ? 'idle' : 'working', currentPrompt: idle.has(input.taskId) ? null : 'prompt', lastActivityMs: clock.ms, deadline: { mode: 'kernel', state: 'armed' } };
      if (op === 'fetch') return { 'ACCEPTANCE_UPDATE.json': { missing: true, reason: 'file_not_found' } };
      if (op === 'bundle') return { base: head, head, sha256: 'c'.repeat(64), path: `/bundles/${input.taskId}.bundle` };
      return { stopped: true };
    },
    cli: async argv => {
      calls.push({ argv });
      if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: lanes.map(l => ({ kernel: 'executor', session_id: `room-${l}` })) } };
      if (argv[1] === 'wait') {
        const stops = calls.filter(c => c.op === 'stop' && !c.served);
        if (stops.length) stops[0].served = true;
        const next = stops.length ? { events: [ev(stops[0].taskId, { type: 'session', value: { state: 'agent_removed' } })] } : waits.shift();
        if (!next) throw new Error('script exhausted'); clock.ms += 60_000; return { events: next.events || [], next_cursor: waits.length + 1, timed_out: !next.events?.length }; }
      return {};
    },
    respond: async input => { calls.push({ op: 'respond', input }); return {}; },
  };
  return { home, clock, calls, waits, idle, options, ev, spec: { home, wakeMinutes: 5, lanes: specs } };
}

test('newSections splits appended sections and ignores earlier bytes', () => {
  const text = '## a\none\n## b\ntwo\n';
  assert.deepEqual(newSections(text, Buffer.byteLength('## a\none\n')), ['## b\ntwo\n']);
  assert.deepEqual(newSections(text, Buffer.byteLength(text)), []);
});

test('round: permission answered, note delivered once, finish collected, hard stop stops', async t => {
  const r = await rig(t);
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
  assert.equal(r.calls.filter(c => c.op === 'status' && c.taskId === 'beta').length, 0);
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

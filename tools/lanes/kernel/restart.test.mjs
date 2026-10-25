// MP-08 / MP-09 / MP-10 / MP-11: restart carries exact rounds and fails clearly at each seam.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { restartWithSandbox } from './restart.mjs';

async function rig(t) {
  const state = await mkdtemp(join(tmpdir(), 'restart-lanes-'));
  t.after(() => rm(state, { recursive: true, force: true }));
  const agents = join(state, 'agents'); await mkdir(join(agents, 'alpha'), { recursive: true });
  const run = { key: 'alpha-r1', round: 'r1', provider: 'claude', launchedAtMs: 123,
    launch: { sessionId: 'session', agentId: 'agent' } };
  const path = join(agents, 'alpha', 'LOOP_RUN.json');
  await writeFile(path, JSON.stringify(run)); await writeFile(join(state, 'loopd.log'), '{"kind":"picked_up","key":"alpha-r1"}\n');
  const calls = [], lines = []; let clock = 0;
  const options = { state, agents, timeoutMs: 2, now: () => clock++, sleep: async () => {},
    preflight: async () => 'chariox(org=…123)', stopLoopd: async () => calls.push('stopLoopd'),
    stop: async () => calls.push('stop'), start: async () => { calls.push('start'); await writeFile(join(state, 'loopd.log'), '{"kind":"picked_up","key":"alpha-r1"}\n'.repeat(2)); },
    inspect: async () => ({ provider: 'claude', permission: 'required', state: 'working', providerActivityAtMs: calls.includes('start') ? 1000 : 0 }), report: line => lines.push(line) };
  return { state, path, run, calls, lines, options };
}

test('one restart preserves session, agent, launch time and existing permission, and reports the account', async t => {
  const r = await rig(t);
  assert.deepEqual((await restartWithSandbox(r.options)).resumed, ['alpha-r1']);
  assert.deepEqual(r.calls, ['stopLoopd', 'stop', 'start']);
  assert.deepEqual(JSON.parse(await readFile(join(r.state, 'kernel-environment.json'), 'utf8')), { IS_SANDBOX: '1' });
  assert.match(r.lines[0], /provider=claude permission=required account=chariox.*resumed=true/);
});

test('unusable account or unsupported runtime leaves every service alone', async t => {
  const r = await rig(t);
  r.options.preflight = async () => { throw new Error('not usable'); };
  await assert.rejects(restartWithSandbox(r.options), /not usable/);
  assert.deepEqual(r.calls, []);
});

test('old picked_up lines cannot confirm a failed restore', async t => {
  const r = await rig(t); r.options.start = async () => r.calls.push('start');
  await assert.rejects(restartWithSandbox(r.options), /rounds did not resume: alpha-r1/);
});

test('a replacement agent or a missing permission level fails verification', async t => {
  for (const broken of ['identity', 'permission', 'driver']) {
    const r = await rig(t); const start = r.options.start;
    r.options.start = async () => { await start();
      if (broken === 'identity') await writeFile(r.path, JSON.stringify({ ...r.run, launch: { agentId: 'replacement' } }));
      if (broken === 'driver') await writeFile(r.path, JSON.stringify({ ...r.run, driverError: { code: 'restore_failed' } }));
    };
    if (broken === 'permission') r.options.inspect = async () => ({ provider: 'claude', state: 'working' });
    await assert.rejects(restartWithSandbox(r.options), /identity changed|no healthy worker|restore_failed/);
  }
});

test('a carried round that finishes before verification is confirmed from its archive', async t => {
  const r = await rig(t); const start = r.options.start;
  r.options.start = async () => {
    await start();
    await writeFile(join(r.options.agents, 'alpha', 'LOOP_RUN.r1.json'), JSON.stringify({ ...r.run, finishReason: 'finished', finishedAtMs: 456, completedTurn: { status: 'completed', completedAtMs: 1000 } }));
    await writeFile(r.path, JSON.stringify({ ...r.run, key: 'alpha-r2', round: 'r2', launchedAtMs: 789 }));
  };
  r.options.inspect = async run => {
    if (run.finishedAtMs) throw new Error('the completed worker has already been removed');
    return { provider: 'claude', permission: 'required', state: 'working' };
  };
  assert.deepEqual((await restartWithSandbox(r.options)).resumed, ['alpha-r1']);
  assert.match(r.lines[0], /completed=true/);
});

for (const failure of ['inspection', 'snapshot']) test(`failed ${failure} restores the running loopd before any kernel stop`, async t => {
  const r = await rig(t);
  r.options.stopLoopd = async () => { r.calls.push('stopLoopd'); return { stopped: true }; };
  r.options.startLoopd = async () => r.calls.push('startLoopd');
  if (failure === 'inspection') r.options.inspect = async () => { throw new Error('inspection failed'); };
  else await mkdir(join(r.state, 'restart-rounds.json'));
  await assert.rejects(restartWithSandbox(r.options), /inspection failed|EISDIR/);
  assert.deepEqual(r.calls, ['stopLoopd', 'startLoopd']);
});

test('a failed precheck leaves an already stopped loopd stopped', async t => {
  const r = await rig(t);
  r.options.stopLoopd = async () => ({ stopped: false });
  r.options.startLoopd = async () => r.calls.push('startLoopd');
  r.options.inspect = async () => { throw new Error('inspection failed'); };
  await assert.rejects(restartWithSandbox(r.options), /inspection failed/);
  assert.deepEqual(r.calls, []);
});

test('receipt pickup without fresh provider activity must never verify recovery', async t => {
  const r = await rig(t);
  r.options.inspect = async () => ({ provider: 'claude', permission: 'required', state: 'working', providerActivityAtMs: 0 });
  await assert.rejects(restartWithSandbox(r.options), /rounds did not resume/);
  assert.equal(r.lines.length, 0);
});

test('a provider that fails after receipt pickup fails restart verification', async t => {
  const r = await rig(t); let inspections = 0;
  r.options.timeoutMs = 10;
  r.options.inspect = async () => ({ provider: 'claude', permission: 'required', state: ++inspections > 2 ? 'error' : 'working' });
  await assert.rejects(restartWithSandbox(r.options), /no healthy worker/);
  assert.equal(r.lines.length, 0);
});

test('replayed pending decisions do not count as fresh provider activity', async t => {
  const r = await rig(t);
  r.options.inspect = async () => ({ provider: 'claude', permission: 'required', state: 'working' });
  const start = r.options.start;
  r.options.start = async () => { await start(); await writeFile(r.path, JSON.stringify({ ...r.run,
    events: [{ kind: 'event', type: 'interaction', state: 'pending', at_ms: 1000, agent_id: 'agent' }] })); };
  await assert.rejects(restartWithSandbox(r.options), /rounds did not resume/);
  assert.equal(r.lines.length, 0);
});

test('a stop timeout after loopd begins stopping restores the prior running service', async t => {
  const r = await rig(t);
  r.options.loopdRunning = async () => true;
  r.options.stopLoopd = async () => { r.calls.push('stopLoopd'); throw new Error('stop timed out'); };
  r.options.startLoopd = async () => r.calls.push('startLoopd');
  await assert.rejects(restartWithSandbox(r.options), /stop timed out.*loopd restored/);
  assert.deepEqual(r.calls, ['stopLoopd', 'startLoopd']);
});

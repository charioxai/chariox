// MP-08 / MP-09 / MP-10 / MP-11: stop admission and the real command preserve round identity.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { requestRoundStop, stopRequestPath, matchesStop } from './round-stop.mjs';

const run = { lane: 'alpha', key: 'alpha-r25', round: 'r25', launchedAtMs: 1000, launch: { kernel: 'executor', sessionId: 'room', agentId: 'agent-current' } };
async function rig(t) {
  const home = await mkdtemp(join(tmpdir(), 'round-stop-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const agents = join(home, 'agents');
  await mkdir(join(agents, 'alpha'), { recursive: true });
  const path = join(agents, 'alpha', 'LOOP_RUN.json');
  await writeFile(path, JSON.stringify(run));
  return { home, agents, path, layout: { home, agents } };
}

test('a stop request survives a driver restart and cannot match a reused key or replacement worker', async t => {
  const r = await rig(t);
  const request = await requestRoundStop('alpha', r);
  const saved = JSON.parse(await readFile(stopRequestPath(r.home, run.key, run.launchedAtMs), 'utf8'));
  assert.deepEqual(saved, request);
  assert.ok(matchesStop(saved, run));
  assert.equal(matchesStop(saved, { ...run, launchedAtMs: 2000 }), false);
  assert.equal(matchesStop(saved, { ...run, launch: { ...run.launch, agentId: 'replacement' } }), false);
  assert.notEqual(stopRequestPath(r.home, run.key, 1000), stopRequestPath(r.home, run.key, 2000));
});

test('stop admission refuses a missing or invalid lane and leaves an already-finished round alone', async t => {
  const r = await rig(t);
  await assert.rejects(requestRoundStop('../alpha', r), { code: 'invalid_lane' });
  await assert.rejects(requestRoundStop('missing', r), { code: 'round_not_running' });
  await assert.rejects(requestRoundStop('alpha', { ...r, expectedRound: 'r24' }), { code: 'round_changed' });
  await writeFile(r.path, JSON.stringify({ ...run, finishedAtMs: 2000, finishReason: 'finished' }));
  assert.deepEqual(await requestRoundStop('alpha', r), { key: run.key, finished: true, finishReason: 'finished' });
});

test('round.mjs stop waits for the exact archived round even when its successor owns the lane', async t => {
  const r = await rig(t);
  const child = spawn(process.execPath, [new URL('./round.mjs', import.meta.url).pathname, 'stop', 'alpha'], {
    env: { ...process.env, CHARIOX_COORDINATOR_HOME: r.home, LOOPD_AGENTS: r.agents }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  t.after(() => { if (Number.isSafeInteger(child.pid) && child.pid > 1 && child.exitCode === null) child.kill('SIGTERM'); });
  let stdout = '', stderr = '';
  child.stdout.on('data', s => stdout += s);
  child.stderr.on('data', s => stderr += s);
  const exited = once(child, 'exit');
  const deadline = Date.now() + 3000;
  while (Date.now() < deadline) {
    try { await readFile(stopRequestPath(r.home, run.key, run.launchedAtMs)); break; }
    catch (error) { if (error.code !== 'ENOENT') throw error; await new Promise(resolve => setTimeout(resolve, 10)); }
  }
  await writeFile(join(r.agents, 'alpha', 'LOOP_RUN.r25.json'), JSON.stringify({ ...run, finishedAtMs: 2000, finishReason: 'stopped', head: 'a'.repeat(40) }));
  await writeFile(r.path, JSON.stringify({ ...run, key: 'alpha-r26', round: 'r26', launchedAtMs: 3000 }));
  const [code] = await exited;
  assert.equal(code, 0, stderr);
  assert.deepEqual(JSON.parse(stdout), { key: run.key, finished: true, finishReason: 'stopped', head: 'a'.repeat(40) });
});

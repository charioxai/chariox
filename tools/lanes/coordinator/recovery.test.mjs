// MP-08 / MP-10 / MP-11: new grants must recover session and agent targets independently.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openInbox, followLaunch } from './events.mjs';
const launch = { kernel: 'executor', sessionId: 'room', operationsSessionId: 'ops', agentId: 'agent' };
test('recovery subscribes the worker agent as well as both sessions', async () => {
  const calls = [];
  await followLaunch(async argv => calls.push(argv), launch);
  assert.ok(calls.some(argv => argv.includes('--agent') && argv.at(-1) === 'agent'));
  assert.equal(calls.filter(argv => !argv.includes('--agent')).length, 2);
});
test('an events_refused grant with only session targets repairs its missing agent target', async t => {
  const home = await mkdtemp(join(tmpdir(), 'inbox-recovery-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  await writeFile(join(home, 'lane.json'), JSON.stringify({ lane: 'lane-r1', launch }));
  const targets = [{ kernel: 'executor', session_id: 'room' }, { kernel: 'executor', session_id: 'ops' }];
  let repaired = false;
  const cli = async argv => {
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'new', targets, kernels: repaired ? [] : [{ kernel: 'executor', error: { code: 'events_refused', message: 'agent target missing' } }] } };
    if (argv[1] === 'subscribe' && argv.includes('--agent')) { repaired = true; targets.push({ kernel: 'executor', session_id: 'room', agent_id: 'agent' }); }
    return {};
  };
  await openInbox(home, cli, 0);
  assert.equal(repaired, true);
});
test('a confirmed missing agent is identified as worker_lost', async () => {
  await assert.rejects(followLaunch(async argv => {
    if (argv.includes('--agent')) throw Object.assign(new Error('missing'), { code: 'agent_not_in_session' });
  }, launch), error => error.code === 'worker_lost' && error.launch.agentId === 'agent');
});

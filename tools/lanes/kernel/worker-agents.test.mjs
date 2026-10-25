// MP-08 / MP-09 / MP-10 / MP-11: subagent definitions reach the worker's account profile.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { syncWorkerAgents } from './worker-agents.mjs';

test('the scout definition is copied under the profile config directory once, and a missing one is reported', async t => {
  const root = await mkdtemp(join(tmpdir(), 'workeragents-')); t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source'), target = join(root, 'claude');
  await mkdir(source); await mkdir(target);
  await writeFile(join(source, 'scout.md'), '---\nname: scout\n---\nread-only helper\n');
  assert.deepEqual(syncWorkerAgents({ source, target }), { written: ['scout'], missing: [] });
  assert.equal(await readFile(join(target, 'agents', 'scout.md'), 'utf8'), '---\nname: scout\n---\nread-only helper\n');
  assert.deepEqual(syncWorkerAgents({ source, target }), { written: [], missing: [] }, 'unchanged: nothing rewritten');
  assert.deepEqual(syncWorkerAgents({ source, target, names: ['scout', 'reviewer'] }), { written: [], missing: ['reviewer'] });
  assert.throws(() => syncWorkerAgents({ source, target, names: ['../x'] }), /invalid agent name/);
});

test('lane child agents use the verified lanes Claude profile by default', async () => {
  const { setWorkerProfileDefault } = await import('./worker-agents.mjs');
  const calls = [];
  await setWorkerProfileDefault({ rpc: async request => calls.push(request), profileId: 'lanes-claude-own' });
  assert.deepEqual(calls, [{ SetDefaultProviderAccountProfile: { provider: 'claude', account_profile: 'lanes-claude-own' } }]);
});

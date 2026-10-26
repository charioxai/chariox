// MP-08 / MP-09 / MP-10 / MP-11: the swap verifier accepts retained pre-fix diagnostics.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('the swap verifier skips old warning lines and requires pickup by the latest service', async t => {
  const root = await mkdtemp(join(tmpdir(), 'pickup-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const agents = join(root, 'agents'); await mkdir(join(agents, 'alpha'), { recursive: true });
  const snapshot = join(root, 'before.json'), log = join(root, 'loopd.log');
  const id = { sessionId: 'room', agentId: 'worker', launchedAtMs: 123 };
  await writeFile(snapshot, JSON.stringify({ 'alpha-r1': id }));
  await writeFile(join(agents, 'alpha', 'LOOP_RUN.json'), JSON.stringify({ launch: id, launchedAtMs: 123 }));
  const prefix = 'Warning: Detected unsettled top-level await at round.mjs:223\n  await main();\n^\nnull\n42\n';
  const run = () => spawnSync(process.execPath, [new URL('./verify-pickup.mjs', import.meta.url).pathname, agents, snapshot, log], { encoding: 'utf8' });
  await writeFile(log, prefix + '{"kind":"started"}\n{"kind":"picked_up","key":"alpha-r1"}\n');
  let result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /same-ids=true picked_up=true/);
  await writeFile(log, prefix + '{"kind":"started"}\n{"kind":"picked_up","key":"alpha-r1"}\n{"kind":"started"}\n');
  result = run();
  assert.equal(result.status, 3, result.stderr);
  assert.match(result.stdout, /FAIL alpha-r1/);
});

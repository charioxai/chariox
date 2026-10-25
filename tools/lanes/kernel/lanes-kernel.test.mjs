// MP-08 / MP-09 / MP-10 / MP-11: the holder's detached exec through the public lanes-kernel.mjs command.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile } from 'node:fs/promises';
import { existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';

const script = new URL('./lanes-kernel.mjs', import.meta.url).pathname;

async function holder(t, extraConfig = {}) {
  const state = await mkdtemp(join(tmpdir(), 'lanes-kernel-')); t.after(() => rm(state, { recursive: true, force: true }));
  await writeFile(join(state, 'config.json'), JSON.stringify({ toolchain: state, ...extraConfig }));
  const env = { PATH: process.env.PATH, HOME: state, LANES_KERNEL_STATE: state };
  const child = spawn(process.execPath, [script, 'holder'], { env, stdio: 'ignore' });
  const exited = once(child, 'exit');
  t.after(async () => { child.kill('SIGTERM'); await exited; });
  for (let i = 0; i < 100 && !existsSync(join(state, 'holder.pid')); i++) await new Promise(resolve => setTimeout(resolve, 50));
  assert.ok(existsSync(join(state, 'holder.pid')), 'the holder is listening');
  return { state, exec: (...args) => spawnSync(process.execPath, [script, 'exec', ...args], { env, encoding: 'utf8' }) };
}

test('exec --detach exits 0 with the pid of a command that started, and keeps its log', async t => {
  const { state, exec } = await holder(t);
  const log = join(state, 'job.log');
  const result = exec('--detach', '--log', log, '--', '/bin/sh', '-c', 'echo detached-output');
  assert.equal(result.status, 0, result.stderr);
  const { pid } = JSON.parse(result.stdout);
  assert.ok(Number.isSafeInteger(pid) && pid > 1);
  for (let i = 0; i < 100 && !readFileSync(log, 'utf8').includes('detached-output'); i++) await new Promise(resolve => setTimeout(resolve, 50));
  assert.match(await readFile(log, 'utf8'), /detached-output/);
});

test('exec --detach fails with the reason when the command cannot start', async t => {
  const { exec } = await holder(t);
  const result = exec('--detach', '--', '/no/such/binary');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /ENOENT/);
  assert.equal(result.stdout.includes('"pid"'), false);
});

test('a state dir with its own agents directory hands it to the commands it runs, so a drill loopd cannot read the real queue', async t => {
  const { exec } = await holder(t, { agents: '/drill/agents' });
  const result = exec('--', '/bin/sh', '-c', 'echo "$LOOPD_AGENTS"');
  assert.equal(result.stdout.trim(), '/drill/agents');
});

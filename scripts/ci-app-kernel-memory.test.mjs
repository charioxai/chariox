import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const root = new URL('../', import.meta.url);
const core = readFileSync(new URL('.github/workflows/ci.yml', root), 'utf8');
const apps = readFileSync(new URL('.github/workflows/apps-components.yml', root), 'utf8');
function memoryStep(workflow) {
  const start = workflow.indexOf('      - name: Add swap for the kernel test build\n');
  assert.ok(start >= 0, 'kernel libtest compilation requires the existing hosted-memory preparation');
  const end = workflow.indexOf('\n      - ', start + 1);
  const block = workflow.slice(start, end < 0 ? undefined : end);
  const script = block.split('        run: |\n')[1]?.split('\n').filter(line => line.startsWith('          ')).map(line => line.slice(10)).join('\n');
  assert.ok(script, 'the memory prerequisite is an executable step');
  return {start, block, script};
}
function runPreparation(t, script, fail = '') {
  const dir = mkdtempSync(join(tmpdir(), 'app-kernel-memory-'));
  t.after(() => rmSync(dir, {recursive: true, force: true}));
  const bin = join(dir, 'bin'), log = join(dir, 'calls.jsonl');
  mkdirSync(bin);
  const fixture = `#!/usr/bin/env node\nconst fs=require('node:fs'),path=require('node:path');\nconst args=[path.basename(process.argv[1]),...process.argv.slice(2)];\nfs.appendFileSync(process.env.CI_TEST_SWAP_LOG,JSON.stringify(args)+'\\n');\nif(args[0]==='sudo'&&args[1]===process.env.CI_TEST_SWAP_FAIL)process.exit(73);\n`;
  for (const name of ['sudo', 'free']) writeFileSync(join(bin, name), fixture, {mode: 0o755});
  const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {env: {...process.env, PATH: `${bin}:${process.env.PATH}`, CI_TEST_SWAP_LOG: log, CI_TEST_SWAP_FAIL: fail}, encoding: 'utf8'});
  const calls = readFileSync(log, 'utf8').trim().split('\n').map(line => JSON.parse(line));
  return {result, calls};
}

test('App kernel compilation uses the existing Linux memory prerequisite before compiling', t => {
  const step = memoryStep(apps);
  assert.match(step.block, /if:\s*runner\.os\s*==\s*'Linux'/);
  assert.ok(step.start < apps.indexOf('      - name: Test kernel App routing and durable writer'));
  const expected = runPreparation(t, memoryStep(core).script);
  const actual = runPreparation(t, step.script);
  assert.equal(expected.result.status, 0, expected.result.stderr);
  assert.equal(actual.result.status, 0, actual.result.stderr);
  assert.deepEqual(actual.calls, expected.calls, 'the two hosted kernel builds prepare the same memory envelope');
});

test('an unavailable App kernel memory prerequisite fails instead of proceeding unswapped', t => {
  const {result, calls} = runPreparation(t, memoryStep(apps).script, 'swapon');
  assert.equal(result.status, 73);
  assert.equal(calls.at(-1)?.[1], 'swapon');
  assert.equal(calls.some(call => call[0] === 'free'), false, 'setup stops at the failed prerequisite');
});

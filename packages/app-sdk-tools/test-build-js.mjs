import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

test('a forbidden Node builtin fails the real packer and preserves the prior backend', async () => {
  assert.ok(process.env.TMPDIR, 'set TMPDIR outside the repository');
  const directory = await mkdtemp(join(process.env.TMPDIR, 'app-packer-test-'));
  try {
    const entry = join(directory, 'app.js'), output = join(directory, 'app.wasm');
    await writeFile(entry, 'import fs from "node:fs"; export const handlers = { register() { return fs.readFileSync("/etc/passwd"); } };');
    await writeFile(output, 'working backend');
    const result = spawnSync(process.execPath, ['packages/app-sdk-tools/build-js.mjs', entry, output], { encoding: 'utf8' });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Could not resolve "node:fs"/);
    assert.equal(await readFile(output, 'utf8'), 'working backend');
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('the real packer refuses GPL and unlicensed bundled dependencies', async () => {
  const directory = await mkdtemp(join(process.env.TMPDIR, 'app-license-test-'));
  try {
    const dependency = join(directory, 'node_modules/unapproved');
    await mkdir(dependency, { recursive: true });
    await writeFile(join(dependency, 'index.js'), 'export const value = 1;');
    const entry = join(directory, 'app.js'), output = join(directory, 'app.wasm');
    await writeFile(entry, 'import {value} from "unapproved"; export const handlers = { register() { return value; } };');
    for (const license of ['GPL-3.0-only', undefined]) {
      await writeFile(join(dependency, 'package.json'), JSON.stringify({ name: 'unapproved', version: '1.0.0', main: 'index.js', license }));
      await writeFile(output, 'working backend');
      const result = spawnSync(process.execPath, ['packages/app-sdk-tools/build-js.mjs', entry, output], { encoding: 'utf8' });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /npm licence review required: unapproved/);
      assert.equal(await readFile(output, 'utf8'), 'working backend');
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

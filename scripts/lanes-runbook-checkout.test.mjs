// MP-08 / MP-10 / MP-11: root tests also run in shallow checkouts.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, cp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

test('runbook inventory check needs no unrelated Git objects', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'dogfood-runbook-'));
  try {
    await mkdir(join(directory, 'scripts'));
    await mkdir(join(directory, 'docs'));
    await copyFile(new URL('./lanes-runbook.test.mjs', import.meta.url), join(directory, 'scripts/lanes-runbook.test.mjs'));
    await copyFile(new URL('../docs/LANES_THROUGH_CHARIOX.md', import.meta.url), join(directory, 'docs/LANES_THROUGH_CHARIOX.md'));
    try { await cp(new URL('./fixtures/lanes-request-api-1144cfc5.rs', import.meta.url), join(directory, 'scripts/fixtures/lanes-request-api-1144cfc5.rs'), { recursive: true }); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    const env = { ...process.env, GIT_CEILING_DIRECTORIES: directory };
    delete env.NODE_TEST_CONTEXT;
    const checked = spawnSync(process.execPath, ['--test', 'scripts/lanes-runbook.test.mjs'], { cwd: directory, encoding: 'utf8', env });
    assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

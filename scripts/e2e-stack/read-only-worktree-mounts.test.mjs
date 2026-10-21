// MP-08 / MP-10 / MP-11: real Git metadata, supplementary harness regressions.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { readOnlyWorktreeMounts } from './read-only-worktree-mounts.mjs';

function repository(run) {
  const root = mkdtempSync(join(tmpdir(), 'chariox-worker-git-'));
  try {
    const primary = join(root, 'primary');
    mkdirSync(primary);
    const git = (...args) => execFileSync('git', ['-C', primary, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
    git('init');
    writeFileSync(join(primary, 'README.md'), 'MP-08 / MP-10 / MP-11 real source\n');
    git('add', 'README.md');
    git('-c', 'user.name=Drill', '-c', 'user.email=drill@example.invalid', 'commit', '-m', 'source');
    const linked = join(root, 'linked');
    git('worktree', 'add', '--detach', linked);
    run({ primary, linked, head: git('rev-parse', 'HEAD') });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('MP-08 / MP-10 / MP-11: linked worker mounts exactly its source and Git common dir read-only', () => repository(({ primary, linked }) => {
  assert.deepEqual(readOnlyWorktreeMounts(linked), [
    '--mount', `type=bind,src=${linked},dst=${linked},readonly`,
    '--mount', `type=bind,src=${primary}/.git,dst=${primary}/.git,readonly`,
  ]);
  assert.deepEqual(readOnlyWorktreeMounts(primary), ['--mount', `type=bind,src=${primary},dst=${primary},readonly`]);
}));

test('MP-08 / MP-10 / MP-11: isolated real worker can read linked HEAD and status', { skip: !process.env.CHARIOX_WORKER_GIT_TEST_IMAGE }, () => repository(({ linked, head }) => {
  const run = (...command) => execFileSync('docker', ['run', '--rm', '--label', 'chariox.lane=amfix', '--cpus', '1', '--memory', '256m', ...readOnlyWorktreeMounts(linked), '--workdir', linked, '--entrypoint', 'git', process.env.CHARIOX_WORKER_GIT_TEST_IMAGE, '-c', `safe.directory=${linked}`, ...command], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  assert.equal(run('rev-parse', 'HEAD'), head);
  assert.equal(run('status', '--porcelain'), '');
}));

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const script = new URL('./check-test-temp.sh', import.meta.url).pathname;

function run(command, env = {}) {
  const parent = mkdtempSync(join(tmpdir(), 'check-test-temp-'));
  const result = spawnSync('sh', [script, 'sh', '-c', command], {
    encoding: 'utf8',
    env: { ...process.env, CHARIOX_KEEP_TEST_TEMP: '', ...env, CHARIOX_TEST_TEMP_PARENT: parent },
  });
  return { ...result, parent, remaining: () => readdirSync(parent) };
}

test('passes and removes its private TMPDIR when the command leaves nothing', () => {
  const outcome = run('mkdir "$TMPDIR/chariox-tests"');
  try {
    assert.equal(outcome.status, 0, outcome.stderr);
    assert.deepEqual(outcome.remaining(), []);
  } finally {
    rmSync(outcome.parent, { recursive: true, force: true });
  }
});

test('fails and removes its private TMPDIR when the command leaves state', () => {
  const outcome = run('touch "$TMPDIR/leftover"');
  try {
    assert.equal(outcome.status, 1);
    assert.match(outcome.stderr, /leftover/);
    assert.deepEqual(outcome.remaining(), []);
  } finally {
    rmSync(outcome.parent, { recursive: true, force: true });
  }
});

test('CHARIOX_KEEP_TEST_TEMP=1 keeps the leftover state and prints where it is', () => {
  const outcome = run('touch "$TMPDIR/leftover"', { CHARIOX_KEEP_TEST_TEMP: '1' });
  try {
    assert.equal(outcome.status, 1);
    const [kept] = outcome.remaining();
    assert.ok(kept, 'the private TMPDIR is gone');
    const dir = join(outcome.parent, kept);
    assert.ok(existsSync(join(dir, 'leftover')), 'the retained file is gone');
    assert.ok(outcome.stderr.includes(dir), `stderr does not name ${dir}: ${outcome.stderr}`);
  } finally {
    rmSync(outcome.parent, { recursive: true, force: true });
  }
});

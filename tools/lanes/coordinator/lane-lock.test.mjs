// MP-08 / MP-09 / MP-10 / MP-11: real competing lock holders.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { tryLaneLock, releaseLaneLock } from './lane-lock.mjs';

test('two contenders race a stale lock; one enters and an old release cannot unlock it', async t => {
  const home = await mkdtemp(join(tmpdir(), 'lane-lock-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const old = await tryLaneLock(home, 'lane', 'steer', () => 0);
  await releaseLaneLock(old);
  const legacy = join(home, 'lane.admission.lock');
  await mkdir(legacy, { recursive: true });
  await writeFile(join(legacy, 'owner'), JSON.stringify({ owner: 'steer', atMs: 0 }));
  const locks = await Promise.all(['loopd', 'steer'].map(owner => tryLaneLock(home, 'lane', owner)));
  const winners = locks.filter(Boolean);
  t.after(async () => { for (const lock of winners) await releaseLaneLock(lock); });
  assert.equal(winners.length, 1, 'exactly one contender enters');
  await releaseLaneLock(old);
  assert.equal(await tryLaneLock(home, 'lane', 'steer'), undefined, 'old release cannot unlock the winner');
});

test('a live holder is never stolen by another loopd or by advancing the clock', async t => {
  const home = await mkdtemp(join(tmpdir(), 'lane-lock-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const lock = await tryLaneLock(home, 'lane', 'loopd', () => 0);
  t.after(() => releaseLaneLock(lock));
  assert.equal(await tryLaneLock(home, 'lane', 'loopd', () => 1e12), undefined);
});

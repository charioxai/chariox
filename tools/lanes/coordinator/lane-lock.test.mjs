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

test('the OS releases a lock when its driver dies, without stealing a live lock', async t => {
  const { spawn } = await import('node:child_process');
  const { once } = await import('node:events');
  const home = await mkdtemp(join(tmpdir(), 'lane-lock-death-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const script = `import { tryLaneLock } from ${JSON.stringify(new URL('./lane-lock.mjs', import.meta.url).href)};
    await tryLaneLock(process.argv[1], 'lane', 'loopd'); console.log('acquired');`;
  const child = spawn(process.execPath, ['--input-type=module', '-e', script, home]);
  const exited = once(child, 'exit');
  await once(child.stdout, 'data');
  assert.equal(await tryLaneLock(home, 'lane', 'steer'), undefined);
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1);
  child.kill('SIGKILL'); await exited;
  let lock;
  for (let attempt = 0; attempt < 50 && !lock; attempt++) {
    lock = await tryLaneLock(home, 'lane', 'steer');
    if (!lock) await new Promise(resolve => setTimeout(resolve, 20));
  }
  assert.ok(lock, 'the dead driver does not leave the lane locked');
  await releaseLaneLock(lock);
});

test('the macOS lock path uses the native exclusive-open flag without spawning flock', async () => {
  const { constants } = await import('node:fs');
  const { tryProcessFileLock } = await import('./process-file-lock.mjs');
  let closed = 0;
  const lock = await tryProcessFileLock('/lane/admission.flock', { platform: 'darwin',
    openFile: async (path, flags, mode) => {
      assert.equal(path, '/lane/admission.flock');
      assert.equal(flags, constants.O_RDWR | constants.O_CREAT | constants.O_NONBLOCK | 0x20);
      assert.equal(mode, 0o600); return { fd: 42, close: async () => { closed++; } };
    }, spawnProcess: () => { throw new Error('macOS must not spawn flock'); },
  });
  await releaseLaneLock(lock); await releaseLaneLock(lock); assert.equal(closed, 1);
  assert.equal(await tryProcessFileLock('/lane/admission.flock', { platform: 'darwin', openFile: async () => { throw Object.assign(new Error('busy'), { code: 'EAGAIN' }); } }), undefined);
});

test('the portable lock also excludes an already running legacy Linux flock holder', { skip: process.platform !== 'linux' }, async t => {
  const { spawn } = await import('node:child_process');
  const { once } = await import('node:events');
  const home = await mkdtemp(join(tmpdir(), 'lane-lock-migration-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const legacy = spawn('flock', ['--exclusive', join(home, 'lane.admission.flock'), 'sh', '-c', 'printf "locked\\n"; cat >/dev/null']);
  const exited = once(legacy, 'close');
  t.after(async () => { legacy.stdin.end(); await exited; });
  await once(legacy.stdout, 'data');
  const lock = await tryLaneLock(home, 'lane', 'steer');
  if (lock) t.after(() => releaseLaneLock(lock));
  assert.equal(lock, undefined, 'a live pre-upgrade collector still owns the lane');
});

test('an unrelated listener cannot block a lane admission', async t => {
  const { createServer } = await import('node:net');
  const { createHash } = await import('node:crypto');
  const { realpath } = await import('node:fs/promises');
  const home = await mkdtemp(join(tmpdir(), 'lane-listener-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const digest = createHash('sha256').update(await realpath(home)).update('\0').update('lane').digest();
  const server = createServer(socket => socket.destroy());
  await new Promise((resolve, reject) => { server.once('error', error => error.code === 'EADDRINUSE' ? resolve() : reject(error)); server.listen(10000 + digest.readUInt32BE(0) % 20000, '127.0.0.1', resolve); });
  t.after(() => server.listening ? new Promise(resolve => server.close(resolve)) : undefined);
  const lock = await tryLaneLock(home, 'lane', 'loopd');
  if (lock) t.after(() => releaseLaneLock(lock));
  assert.ok(lock, 'lane admission must depend only on the lane lock file');
});

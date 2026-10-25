// MP-08 / MP-09 / MP-10 / MP-11: serialize direct prompts with collection for the entire operation.
import { stat, readFile, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { LaneError } from '../lanes.mjs';

// flock owns the lock until its stdin closes, including when this process dies. Never unlink the
// file: replacing its inode would let two holders enter. The returned handle releases only its fd.
export async function tryLaneLock(home, name, owner, now = Date.now) {
  const child = spawn('flock', ['--exclusive', '--nonblock', join(home, `${name}.admission.flock`),
    'sh', '-c', 'printf "locked\\n"; cat >/dev/null'], { stdio: ['pipe', 'pipe', 'pipe'] });
  child.stdin.on('error', () => {});
  const exited = new Promise((resolve, reject) => { child.once('error', reject); child.once('close', code => resolve(code)); });
  let stderr = '';
  child.stderr.on('data', chunk => { stderr += chunk; });
  const acquired = await Promise.race([
    new Promise(resolve => child.stdout.once('data', () => resolve(true))),
    exited.then(code => { if (code === 1) return false; throw new Error(`lane lock failed (${code}): ${stderr.trim()}`); }),
  ]);
  if (!acquired) return undefined;
  const handle = { child, exited };
  try {
    // Bridge directory locks left by the previous driver. Migration is itself under flock.
    const dir = join(home, `${name}.admission.lock`);
    const held = await stat(dir).catch(error => { if (error.code !== 'ENOENT') throw error; });
    if (held) {
      const holder = await readFile(join(dir, 'owner'), 'utf8').then(JSON.parse, () => ({ atMs: held.mtimeMs }));
      if (now() - holder.atMs <= (holder.owner === 'loopd' ? 15 : 3) * 60_000) {
        await releaseLaneLock(handle); return undefined;
      }
      await rm(dir, { recursive: true, force: true });
    }
    return handle;
  } catch (error) { await releaseLaneLock(handle); throw error; }
}

export async function releaseLaneLock(handle) {
  handle.child.stdin.end();
  await handle.exited;
}

// Waits for a busy lock (a collection in progress) up to waitMs, then says what to do.
export async function withLaneLock(home, name, owner, fn, { now = Date.now, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)), waitMs = 5 * 60_000 } = {}) {
  const started = now();
  let dir;
  while (!(dir = await tryLaneLock(home, name, owner, now))) {
    if (now() - started >= waitMs) throw new LaneError('lane_busy', `${name} is being collected or prompted by another step; retry in a minute.`);
    await sleep(250);
  }
  try { return await fn(); } finally { await releaseLaneLock(dir); }
}

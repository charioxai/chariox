// MP-08 / MP-09 / MP-10 / MP-11: serialize direct prompts with collection for the entire operation.
import { stat, readFile, rm, realpath } from 'node:fs/promises';
import { createServer } from 'node:net';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { LaneError } from '../lanes.mjs';

// A loopback listener is an OS-owned lock on Linux and macOS: process death releases it.
// Hash collisions conservatively report busy; they can never admit two holders. No pathname is
// unlinked on release, so an old handle cannot release its successor's lock.
export async function tryLaneLock(home, name, owner, now = Date.now) {
  const digest = createHash('sha256').update(await realpath(home)).update('\0').update(name).digest();
  const port = 20000 + digest.readUInt32BE(0) % 40000;
  const server = createServer(socket => socket.destroy());
  const acquired = await new Promise((resolve, reject) => {
    server.once('error', error => { if (error.code === 'EADDRINUSE') resolve(false); else reject(error); });
    server.listen({ host: '127.0.0.1', port, exclusive: true }, () => resolve(true));
  });
  if (!acquired) return undefined;
  const handle = { server };
  try {
    // Bridge directory locks left by the previous driver. Migration is itself under the listener lock.
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
  handle.release ??= new Promise((resolve, reject) => handle.server.close(error => error ? reject(error) : resolve()));
  await handle.release;
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

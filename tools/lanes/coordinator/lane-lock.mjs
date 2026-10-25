// MP-08 / MP-09 / MP-10 / MP-11: one lock per lane between the coordinator's direct prompts and loopd's collection of that lane.
// A prompt admitted while loopd reads the worker idle, bundles it and stops it would be destroyed by that stop, so both sides
// take this lock for the whole of their step. The lock is a directory (mkdir is atomic) in the coordinator home.
import { mkdir, stat, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { LaneError } from '../lanes.mjs';

// A holder that died leaves its lock behind: a steer's lock is short, a collection's can take minutes, and loopd (the only
// collector) may take over its own lock from a previous incarnation.
const STALE_MS = { steer: 3 * 60_000, loopd: 15 * 60_000 };

export async function tryLaneLock(home, name, owner, now = Date.now) {
  const dir = join(home, `${name}.admission.lock`);
  for (let attempt = 0; attempt < 2; attempt++) {
    try { await mkdir(dir); await writeFile(join(dir, 'owner'), JSON.stringify({ owner, atMs: now() }), { mode: 0o600 }); return dir; }
    catch (error) {
      if (error.code !== 'EEXIST') throw error;
      const held = await stat(join(dir, 'owner')).catch(() => stat(dir).catch(() => undefined));
      if (!held) continue;
      const holder = await readFile(join(dir, 'owner'), 'utf8').then(JSON.parse, () => ({ owner: 'unknown', atMs: held.mtimeMs }));
      const stale = now() - holder.atMs > (STALE_MS[holder.owner] ?? STALE_MS.steer);
      if (!stale && !(owner === 'loopd' && holder.owner === 'loopd')) return undefined;
      await rm(dir, { recursive: true, force: true });
    }
  }
  return undefined;
}

export const releaseLaneLock = dir => rm(dir, { recursive: true, force: true });

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

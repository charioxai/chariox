// MP-11: signal only unreaped children and verified descendants, never a group/name.
import { readFile, readdir } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
export const validOwnedPid = pid => Number.isSafeInteger(pid) && pid > 1 && pid <= 2147483647;
async function identity(pid) {
  if (!validOwnedPid(pid)) return null;
  try {
    const stat = await readFile(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    return { pid, parent: Number(fields[1]), started: fields[19] };
  } catch (error) { if (error.code === 'ENOENT' || error.code === 'ESRCH') return null; throw error; }
}
export async function processIdentity(pid, parent = process.pid) {
  const item = await identity(pid);
  return item?.parent === parent ? item : null;
}
export async function descendants(roots) {
  const processes = (await Promise.all((await readdir('/proc')).filter(name => /^\d+$/.test(name)).map(name => identity(Number(name))))).filter(Boolean);
  const owned = new Map(roots.map(item => [item.pid, item]));
  for (let changed = true; changed;) {
    changed = false;
    for (const item of processes) if (!owned.has(item.pid) && owned.has(item.parent)) { owned.set(item.pid, item); changed = true; }
  }
  return [...owned.values()].reverse();
}
export async function signalOwned(item, signal) {
  if (!validOwnedPid(item?.pid)) throw new Error('MP-11: invalid owned desktop PID');
  const current = await identity(item.pid);
  if (!current || current.started !== item.started || current.parent !== item.parent) return false;
  try { process.kill(item.pid, signal); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  return true;
}
export async function settleOwned(children) {
  const roots = children.filter(item => item.identity && item.child.exitCode === null && item.child.signalCode === null).map(item => item.identity);
  const owned = await descendants(roots);
  for (const item of owned) await signalOwned(item, 'SIGTERM');
  for (let n = 0; n < 40 && children.some(({child}) => child.exitCode === null && child.signalCode === null); n++) await delay(25);
  for (const item of owned) await signalOwned(item, 'SIGKILL');
  await Promise.all(children.map(({child}) => child.exitCode !== null || child.signalCode !== null ? undefined : new Promise(resolve => child.once('exit', resolve))));
}

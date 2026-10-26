// MP-11: signal only unreaped children and verified descendants, never a group/name.
import { readFile, readdir } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
export const validOwnedPid = pid => Number.isSafeInteger(pid) && pid > 1 && pid <= 2147483647;
async function identity(pid, read = readFile) {
  if (!validOwnedPid(pid)) return null;
  try {
    const stat = await read(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    return { pid, parent: Number(fields[1]), started: fields[19], state: fields[0] };
  } catch (error) { if (error.code === 'ENOENT' || error.code === 'ESRCH') return null; throw error; }
}
export async function processIdentity(pid, parent = process.pid) {
  const item = await identity(pid);
  return item?.parent === parent ? item : null;
}
export async function descendants(roots, { read = readFile, list = readdir } = {}) {
  const processes = (await Promise.all((await list('/proc')).filter(name => /^\d+$/.test(name)).map(name => identity(Number(name), read)))).filter(Boolean);
  const owned = new Map(roots.filter(item=>processes.some(current=>current.pid===item.pid && current.started===item.started && (current.parent===item.parent || (item.parent>1 && current.parent===1)))).map(item => [item.pid, item]));
  for (let changed = true; changed;) {
    changed = false;
    for (const item of processes) if (!owned.has(item.pid) && owned.has(item.parent)) { owned.set(item.pid, item); changed = true; }
  }
  return [...owned.values()].reverse();
}
export async function signalOwned(item, signal) {
  if (!validOwnedPid(item?.pid)) throw new Error('MP-11: invalid owned desktop PID');
  const current = await identity(item.pid);
  if (!current || current.started !== item.started || (current.parent !== item.parent && !(item.parent>1 && current.parent===1))) return false;
  try { process.kill(item.pid, signal); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  return true;
}
export async function settleOwned(children, known = []) {
  const roots = children.filter(item => item.identity && item.child.exitCode === null && item.child.signalCode === null).map(item => item.identity);
  // Previously verified descendants can outlive a graceful parent exit.
  const owned = [...new Map([...known,...await descendants(roots)].map(item=>[item.pid,item])).values()];
  for (const item of owned) await signalOwned(item, 'SIGTERM');
  for (let n = 0; n < 40 && children.some(({child}) => child.exitCode === null && child.signalCode === null); n++) await delay(25);
  for (const item of owned) await signalOwned(item, 'SIGKILL');
  await Promise.all(children.map(({child}) => child.exitCode !== null || child.signalCode !== null ? undefined : new Promise(resolve => child.once('exit', resolve))));
  // Readiness pipes can be inherited by daemon descendants. Retire all owned
  // parent descriptors even after exit, so no reader keeps the kernel alive.
  for(const {child} of children)for(const stream of child.stdio??[])stream?.destroy?.();
}

export async function isOwnedAlive(item) {
  const current=await identity(item?.pid);
  return Boolean(current && current.started===item.started && current.state!=='Z');
}

// MP-08/MP-10/MP-11: conservative observation-only subset; never use for cleanup.
// Linux task children can omit entries during exits. Unknown windows remain masked.
export async function observedDescendants(roots, { read = readFile, list = readdir } = {}) {
 const owned=new Map(),queue=[];
 const discard=pid=>{const removed=new Set([pid]);for(let changed=true;changed;){changed=false;for(const item of owned.values())if(removed.has(item.parent)&&!removed.has(item.pid)){removed.add(item.pid);changed=true}}for(const id of removed)owned.delete(id)};
 const matches=(current,item)=>current&&current.started===item.started&&(current.parent===item.parent||(item.parent>1&&current.parent===1));
 for(const root of roots){const current=await identity(root?.pid, read);if(matches(current,root)){owned.set(current.pid,current);queue.push(current)}}
 for(let index=0;index<queue.length;index++){
  const parent=queue[index];if(!owned.has(parent.pid))continue;let tasks;
  try{tasks=(await list(`/proc/${parent.pid}/task`)).filter(name=>/^\d+$/.test(name))}catch(error){if(['ENOENT','ESRCH','ENOTDIR'].includes(error.code)){discard(parent.pid);continue;}throw error}
  if(tasks.length>1024||owned.size>8192)throw Error('MP-11 observation process bound');
  const childLists=await Promise.all(tasks.map(async tid=>{try{return await read(`/proc/${parent.pid}/task/${tid}/children`,'utf8')}catch(error){if(['ENOENT','ESRCH','ENOTDIR'].includes(error.code))return '';throw error}}));
  if(!matches(await identity(parent.pid, read),parent)){discard(parent.pid);continue;}
  const ids=[...new Set(childLists.flatMap(list=>list.trim().split(/\s+/).map(Number)).filter(validOwnedPid))];
  if(ids.length>8192-owned.size)throw Error('MP-11 observation process bound');
  const children=await Promise.all(ids.map(pid=>identity(pid, read)));
  if(!matches(await identity(parent.pid, read),parent)){discard(parent.pid);continue;}
  for(const child of children)if(child&&child.parent===parent.pid&&!owned.has(child.pid)){owned.set(child.pid,child);queue.push(child)}
 }
 return [...owned.values()].reverse();
}

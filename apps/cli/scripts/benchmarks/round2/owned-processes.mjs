// MP-08 / MP-10 / MP-11: Linux metadata only; never read process credentials.
import { readdir, readFile } from 'node:fs/promises'

async function readProcesses() {
  const rows = await Promise.all((await readdir('/proc')).filter(name => /^\d+$/.test(name)).map(async name => {
    try {
      const stat = await readFile(`/proc/${name}/stat`, 'utf8')
      const fields = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/)
      return { pid: Number(name), parent: Number(fields[1]), group: Number(fields[2]), session: Number(fields[3]), start: fields[19] }
    } catch (error) {
      if (['ENOENT', 'ESRCH'].includes(error.code)) return null
      throw error
    }
  }))
  return rows.filter(Boolean)
}

export function createOwnedProcessSignaler({ ownerPid = process.pid, readProcesses: read = readProcesses, sendSignal = (pid, signal) => process.kill(pid, signal) } = {}) {
  const owned = new WeakMap()
  return async (child, signal, { detached = false } = {}) => {
    const pid = child?.pid
    const safe = value => Number.isSafeInteger(value) && value > 1 && value !== ownerPid
    if (!safe(pid)) throw new Error('MP-10 unsafe cleanup PID')
    if (!['SIGTERM', 'SIGKILL'].includes(signal)) throw new Error('MP-10 unsafe cleanup signal')
    const rows = await read(), byPid = new Map(rows.map(row => [row.pid, row]))
    const known = owned.get(child) ?? new Map(), leader = byPid.get(pid)
    const identity = row => `${row.start}:${row.group}:${row.session}`
    if (leader && (known.has(pid) ? known.get(pid) !== identity(leader) : leader.parent !== ownerPid)) throw new Error('MP-10 cleanup ownership changed')
    if (leader && detached && (leader.group !== pid || leader.session !== pid)) throw new Error('MP-10 cleanup group ownership missing')
    const targets = detached ? rows.filter(row => row.group === pid) : leader ? [leader] : []
    const depth = row => {
      let current = row, level = 0
      const seen = new Set()
      while (current && !seen.has(current.pid)) {
        if (known.get(current.pid) === identity(current) || current.pid === pid && leader?.parent === ownerPid) return level
        seen.add(current.pid); current = byPid.get(current.parent); level++
      }
      throw new Error('MP-10 cleanup descendant ownership missing')
    }
    // Validate every member before sending anything; retain exact identities for
    // a signal-resistant descendant reparented after the leader exits.
    const ordered = targets.map(row => {
      if (!safe(row.pid) || !row.start) throw new Error('MP-10 unsafe cleanup member PID')
      return { row, depth: depth(row) }
    }).sort((a, b) => b.depth - a.depth)
    for (const { row } of ordered) known.set(row.pid, identity(row))
    owned.set(child, known)
    for (const { row } of ordered) {
      const current = (await read()).find(item => item.pid === row.pid)
      if (!current) continue
      if (identity(current) !== known.get(row.pid)) throw new Error('MP-10 cleanup ownership changed before signal')
      // Positive PIDs only: no implicit current group, broadcast, or PID 1.
      if (!safe(current.pid)) throw new Error('MP-10 unsafe cleanup signal PID')
      try { sendSignal(current.pid, signal) } catch (error) { if (error.code !== 'ESRCH') throw error }
    }
  }
}

export const signalOwnedProcess = createOwnedProcessSignaler()

// Wait for Node's child exit acknowledgement before recording cleanup success.
export async function stopOwnedProcess(child, { detached = false, graceMs = 5000, killMs = 5000 } = {}) {
  const exited = () => child.exitCode !== null || child.signalCode !== null
  const wait = async timeout => {
    const deadline = performance.now() + timeout
    while (!exited() && performance.now() < deadline) await new Promise(resolve => setTimeout(resolve, 25))
    return exited()
  }
  await signalOwnedProcess(child, 'SIGTERM', { detached })
  await wait(graceMs)
  // Escalate verified orphan descendants even when the leader already exited.
  await signalOwnedProcess(child, 'SIGKILL', { detached })
  return await wait(killMs)
}

// MP-08/MP-10/MP-11: verify every member before signaling a spawned drill group.
import { execFileSync } from 'node:child_process'

function processSnapshot() {
  return execFileSync('ps', ['-eo', 'pid=,ppid=,pgid=,lstart='], { encoding: 'utf8' })
    .trim().split('\n').filter(Boolean).map(line => {
      const fields = line.trim().split(/\s+/)
      return { pid: Number(fields[0]), ppid: Number(fields[1]), pgid: Number(fields[2]), started: fields.slice(3).join(' ') }
    })
}

export function createOwnedDrillProcessGroup(child, {
  snapshot = processSnapshot,
  signal = (pid, value) => process.kill(pid, value),
} = {}) {
  const pid = child?.pid
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error('MP-08/MP-10/MP-11 unsafe cleanup PID')
  const owned = new Map()
  function members(rows, initial = false) {
    const group = rows.filter(row => row.pgid === pid)
    const root = rows.find(row => row.pid === pid)
    if (initial && (!root || root.pgid !== pid || root.ppid !== process.pid)) {
      throw new Error('MP-08/MP-10/MP-11 missing owned process group root')
    }
    const byPid = new Map(rows.map(row => [row.pid, row]))
    for (const row of group) {
      if (!Number.isSafeInteger(row.pid) || row.pid <= 1 || !row.started) {
        throw new Error('MP-08/MP-10/MP-11 unsafe cleanup PID')
      }
      const previous = owned.get(row.pid)
      if (previous !== undefined && previous !== row.started) {
        throw new Error('MP-08/MP-10/MP-11 reused process group PID')
      }
      if (previous === row.started) continue
      let current = row
      const seen = new Set()
      while (current && !seen.has(current.pid)) {
        seen.add(current.pid)
        if (current.pid === pid && current.ppid === process.pid
          && (initial || owned.get(pid) === current.started)) break
        current = byPid.get(current.ppid)
      }
      if (current?.pid !== pid || current.ppid !== process.pid
        || (!initial && owned.get(pid) !== current.started)) {
        throw new Error('MP-08/MP-10/MP-11 foreign process group member')
      }
    }
    for (const row of group) owned.set(row.pid, row.started)
    return group
  }
  members(snapshot(), true)
  return {
    exists: () => members(snapshot()).length > 0,
    signal(value) {
      // A settled root may leave previously verified descendants. Newly joined
      // processes must still prove ancestry to the original live root.
      if (members(snapshot()).length === 0) return false
      try { signal(-pid, value); return true }
      catch (error) { if (error?.code === 'ESRCH') return false; throw error }
    },
  }
}

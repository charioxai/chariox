// MP-11: process ownership is a PID + start-time identity, never a name or PGID alone.
import { spawn } from 'node:child_process'
import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

function validId(id) {
  if (!Number.isSafeInteger(id) || id <= 1) throw new Error('invalid owned process identity')
  return id
}

// Only public process metadata is read; arguments/environment are never inspected.
export function processSnapshot() {
  if (process.platform === 'linux') {
    const rows = []
    for (const name of readdirSync('/proc')) {
      if (!/^[0-9]+$/.test(name)) continue
      try {
        const text = readFileSync(`/proc/${name}/stat`, 'utf8')
        const fields = text.slice(text.lastIndexOf(')') + 2).trim().split(/\s+/)
        rows.push({ pid: Number(name), ppid: Number(fields[1]), pgid: Number(fields[2]), sid: Number(fields[3]), state: fields[0], start: fields[19] })
      } catch (error) { if (!['ENOENT', 'ESRCH'].includes(error.code)) throw error }
    }
    return rows
  }
  if (process.platform !== 'darwin') throw new Error('owned group inspection is unsupported on this platform')
  return execFileSync('/bin/ps', ['-axo', 'pid=,ppid=,pgid=,sess=,lstart='], {
    encoding: 'utf8', env: { PATH: '/usr/bin:/bin', LC_ALL: 'C' }, maxBuffer: 4 * 1024 * 1024,
  }).trim().split('\n').filter(Boolean).map(line => {
    const [pid, ppid, pgid, session, ...start] = line.trim().split(/\s+/)
    return { pid: Number(pid), ppid: Number(ppid), pgid: Number(pgid), session, start: start.join(' ') }
  })
}

export function createOwnedSignalGuard({ snapshot = processSnapshot, send = (id, signal) => process.kill(id, signal) } = {}) {
  const owned = new Map()
  const roots = new Set()
  const sessions = new Map()
  const matches = row => row && typeof row.start === 'string' && row.start.length > 0 && owned.get(row.pid) === row.start
  function refresh(rows = snapshot()) {
    // A reused PID cannot become a new parent; only current recorded identities
    // can admit descendants. After exit, an unknown/reparented member is refused.
    let changed = true
    while (changed) {
      changed = false
      for (const row of rows) {
        if (owned.has(row.pid) || row.pid <= 1 || !row.start) continue
        // A live recorded setsid leader pins an exclusive run session. POSIX
        // forbids an outside process joining it. This verifies fast reparented
        // descendants without treating an exited/reused session number as proof.
        const inOwnedSession = rows.some(root => roots.has(root.pid) && root.sid === root.pid
          && root.sid === row.sid && matches(root))
          || rows.some(root => sessions.has(root.pid) && sessions.get(root.pid) === row.session
            && root.session === row.session && matches(root))
        if (inOwnedSession || rows.some(parent => parent.pid === row.ppid && matches(parent))) {
          owned.set(row.pid, row.start); changed = true
        }
      }
    }
    return rows
  }
  function record(id, { sessionLeader = false } = {}) {
    validId(id)
    const rows = snapshot()
    const row = rows.find(row => row.pid === id)
    if (!row || typeof row.start !== 'string' || !row.start) throw new Error('owned process start identity unavailable')
    if (owned.has(id) && !matches(row)) throw new Error('unowned reused process identity')
    owned.set(id, row.start); roots.add(id)
    // spawn(detached) creates a new session on macOS as well as Linux. The
    // BSD ps session token binds its live, recorded leader to orphaned members.
    if (sessionLeader && row.pgid === id && row.session && row.session !== '-') sessions.set(id, row.session)
    refresh(rows)
    return id
  }
  function group(id, signal) {
    validId(id)
    const rows = refresh()
    const members = rows.filter(row => row.pgid === id)
    if (!members.length) return false
    if (!roots.has(id) && !members.some(row => row.pid === id && matches(row))) throw new Error('unowned process group')
    if (members.some(row => !matches(row))) throw new Error('unowned process group member')
    // Snapshot and identity verification immediately precede the sole signal call.
    send(-id, signal)
    return true
  }
  function pid(id, signal) {
    validId(id)
    const row = refresh().find(row => row.pid === id)
    if (!row) return false
    if (!matches(row)) throw new Error('unowned process identity')
    send(id, signal)
    return true
  }
  return { record, refresh, group, pid }
}

const guard = createOwnedSignalGuard()
const children = new WeakMap()
const watching = new Set()
let timer
export function trackOwnedProcess(child, { group = false } = {}) {
  if (!child?.pid) return child // spawn error: there is no signalable process.
  guard.record(child.pid, { sessionLeader: group })
  children.set(child, child.pid)
  if (group) {
    watching.add(child)
    if (!timer) {
      timer = setInterval(() => { try { guard.refresh() } catch { /* signal time fails closed */ } }, 25)
      timer.unref()
    }
    child.once('close', () => {
      watching.delete(child)
      if (!watching.size) { clearInterval(timer); timer = undefined }
    })
  }
  return child
}
export function spawnOwned(command, args, options) {
  const { retainLeader = false, ...spawnOptions } = options ?? {}
  if (retainLeader && spawnOptions.detached && process.platform !== 'win32') {
    return trackOwnedProcess(spawn(process.execPath,
      [fileURLToPath(new URL('./owned-process-anchor.mjs', import.meta.url)), command, ...args], spawnOptions), { group: true })
  }
  return trackOwnedProcess(spawn(command, args, spawnOptions), { group: spawnOptions.detached === true })
}
function identity(childOrId) {
  return validId(typeof childOrId === 'object' && childOrId !== null ? children.get(childOrId) : childOrId)
}
export function signalOwnedProcessGroup(childOrId, signal = 'SIGTERM') {
  try { return guard.group(identity(childOrId), signal) }
  catch (error) { if (error.code === 'ESRCH') return false; throw error }
}
export function signalOwnedProcess(childOrId, signal = 'SIGTERM') {
  try { return guard.pid(identity(childOrId), signal) }
  catch (error) { if (error.code === 'ESRCH') return false; throw error }
}
export function ownedProcessGroupExists(childOrId) {
  return signalOwnedProcessGroup(childOrId, 0)
}

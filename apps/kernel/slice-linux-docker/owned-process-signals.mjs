// MP-11: process ownership is a PID + start-time identity, never a name or PGID alone.
import { spawn, spawnSync } from 'node:child_process'
import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

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
  const inspection = spawnSync('/bin/ps', ['-axo', 'pid=,ppid=,pgid=,lstart='], {
    encoding: 'utf8', env: { PATH: '/usr/bin:/bin', LC_ALL: 'C' }, maxBuffer: 4 * 1024 * 1024,
  })
  if (inspection.error) throw inspection.error
  if (inspection.status !== 0) throw new Error('owned process inspection failed')
  validId(inspection.pid)
  return inspection.stdout.trim().split('\n').filter(Boolean).map(line => {
    const [pid, ppid, pgid, ...start] = line.trim().split(/\s+/)
    return { pid: Number(pid), ppid: Number(ppid), pgid: Number(pgid), start: start.join(' ') }
  })
  // MP-11: ps inherits our group and lists itself, but has already been reaped.
  // Exclude only this invocation's exact PID, never its name or other children.
  .filter(row => row.pid !== inspection.pid)
}

export function createOwnedSignalGuard({ snapshot = processSnapshot, send = (id, signal) => process.kill(id, signal) } = {}) {
  const owned = new Map()
  const roots = new Map()
  const handles = new WeakMap()
  const matches = row => row && typeof row.start === 'string' && row.start.length > 0
    && owned.get(row.pid)?.start === row.start
  function refresh(rows = snapshot()) {
    const byPid = new Map(rows.map(row => [row.pid, row]))
    // Retire missing/reused identities before discovering new descendants.
    for (const [id, entry] of owned) {
      if (byPid.get(id)?.start !== entry.start) owned.delete(id)
    }
    for (const [id, root] of roots) {
      if (byPid.get(id)?.start !== root.start) root.retired = true
    }
    const survivingGenerations = new Set([...owned.values()].map(entry => entry.generation))
    for (const [id, root] of roots) {
      if (root.retired && !survivingGenerations.has(root)) roots.delete(id)
    }
    // MP-11: index this snapshot once; polling must not scan the entire host
    // process table again for every unrelated process or descendant.
    const bySid = new Map()
    for (const root of roots.values()) {
      if (root.retired) continue
      const leader = byPid.get(root.pid)
      if (leader.sid === leader.pid && !bySid.has(leader.sid)) bySid.set(leader.sid, root)
    }
    const descendants = new Map()
    const queue = [...owned.keys()]
    for (const row of rows) {
      if (owned.has(row.pid) || row.pid <= 1 || !row.start) continue
      const root = bySid.get(row.sid)
      if (root) {
        owned.set(row.pid, { start: row.start, generation: root })
        queue.push(row.pid)
      } else {
        const children = descendants.get(row.ppid) ?? []
        children.push(row)
        descendants.set(row.ppid, children)
      }
    }
    for (let index = 0; index < queue.length; index++) {
      const parentId = queue[index]
      if (!matches(byPid.get(parentId))) continue
      for (const row of descendants.get(parentId) ?? []) {
        if (owned.has(row.pid)) continue
        owned.set(row.pid, { start: row.start, generation: owned.get(parentId).generation })
        queue.push(row.pid)
      }
    }
    return rows
  }
  function record(id, { freshLaunch = false, expectedStart } = {}) {
    validId(id)
    const rows = snapshot()
    const row = rows.find(row => row.pid === id)
    if (!row || typeof row.start !== 'string' || !row.start) throw new Error('owned process start identity unavailable')
    if (expectedStart !== undefined && row.start !== expectedStart) throw new Error('unowned reused process identity')
    const previous = roots.get(id)
    if (previous && previous.start !== row.start && !freshLaunch) throw new Error('unowned reused process identity')
    refresh(rows)
    const root = previous?.start === row.start ? previous : { pid: id, start: row.start, retired: false }
    // MP-11: BSD sess is not ownership evidence (macOS reports 0 globally).
    // Darwin descendants must be observed through PID/PPID and start identity.
    handles.set(root, { id, root, start: root.start })
    roots.set(id, root)
    owned.set(id, { start: row.start, generation: root })
    refresh(rows)
    return root
  }
  function target(value) {
    validId(typeof value === 'object' && value !== null ? value.pid : value)
    const target = typeof value === 'object' && value !== null ? handles.get(value) : undefined
    if (!target) throw new Error('owned process generation handle required')
    return target
  }
  function pidHandle(value, id) {
    validId(id)
    const { root } = target(value)
    const row = refresh().find(row => row.pid === id)
    if (!matches(row) || owned.get(id)?.generation !== root) throw new Error('unowned process descendant generation')
    const handle = Object.freeze({ pid: id })
    handles.set(handle, { id, root, start: row.start })
    return handle
  }
  function processes(value) {
    const { root } = target(value)
    return refresh().filter(row => matches(row) && owned.get(row.pid)?.generation === root)
      .map(row => {
        const handle = Object.freeze({ pid: validId(row.pid) })
        handles.set(handle, { id: row.pid, root, start: row.start })
        return handle
      })
  }
  function groupHandle(value, id) {
    validId(id)
    const { root } = target(value)
    const members = refresh().filter(row => row.pgid === id)
    if (!members.some(row => matches(row) && owned.get(row.pid)?.generation === root)) {
      throw new Error('unowned process subgroup generation')
    }
    const handle = Object.freeze({ pid: id })
    handles.set(handle, { id, root })
    return handle
  }
  function groups(value) {
    const { root } = target(value)
    const ids = [...new Set(refresh().filter(row => matches(row)
      && owned.get(row.pid)?.generation === root).map(row => row.pgid))]
    // MP-11: collection uses one snapshot; a member may exit immediately after
    // collection. group() still verifies every live member afresh before send.
    return ids.map(id => {
      const handle = Object.freeze({ pid: validId(id) })
      handles.set(handle, { id, root })
      return handle
    })
  }
  function group(value, signal) {
    const { id, root } = target(value)
    const rows = refresh()
    const members = rows.filter(row => row.pgid === id)
    if (!members.length) return false
    if (!root || members.some(row => !matches(row) || owned.get(row.pid)?.generation !== root)) throw new Error('unowned process group member')
    // Snapshot and identity verification immediately precede the sole signal call.
    send(-id, signal)
    return true
  }
  function pid(value, signal) {
    const { id, root, start } = target(value)
    const row = refresh().find(row => row.pid === id)
    if (!row) return false
    if (!root || !matches(row) || row.start !== start || owned.get(id)?.generation !== root) throw new Error('unowned process identity')
    send(id, signal)
    return true
  }
  function retire(root) {
    if (roots.get(root?.pid) === root) root.retired = true
    refresh()
  }
  return { record, refresh, group, pid, retire, groupHandle, groups, pidHandle, processes }
}

const guard = createOwnedSignalGuard()
const children = new WeakMap()
const watching = new Set()
let timer
export function trackOwnedProcess(child, { group = false } = {}) {
  if (!child?.pid) return child // spawn error: there is no signalable process.
  const generation = guard.record(child.pid, { freshLaunch: true })
  children.set(child, generation)
  child.once('close', () => { try { guard.retire(generation) } catch { /* signal time fails closed */ } })
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
function identity(childOrHandle) {
  if (typeof childOrHandle === 'object' && childOrHandle !== null) return children.get(childOrHandle) ?? childOrHandle
  validId(childOrHandle)
  throw new Error('owned process generation handle required')
}
// Numeric subgroup IDs are admitted only with an explicit launch generation.
export function ownedProcessGroupHandle(childOrHandle, groupId) {
  return guard.groupHandle(identity(childOrHandle), groupId)
}
export function ownedProcessGroupHandles(childOrHandle) {
  return guard.groups(identity(childOrHandle))
}
export function ownedProcessHandles(childOrHandle) {
  return guard.processes(identity(childOrHandle))
}
// Captured launch metadata must come from the run's launch path, never a lookup at cleanup time.
export function registerCapturedProcessGroup({ pid, startedAtTicks, processGroupId }) {
  if (validId(pid) !== processGroupId || typeof startedAtTicks !== 'string' || !startedAtTicks) {
    throw new Error('invalid captured process group identity')
  }
  return guard.record(pid, { freshLaunch: true, expectedStart: startedAtTicks })
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

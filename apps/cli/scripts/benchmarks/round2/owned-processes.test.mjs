// MP-08 / MP-10 / MP-11: never signal system or foreign processes during cleanup.
import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createOwnedProcessSignaler, stopOwnedProcess } from './owned-processes.mjs'

const member = (pid, parent, group = 20, start = '100') => ({ pid, parent, group, session: group, start })
function fixture(rows) {
  const calls = []
  const signal = createOwnedProcessSignaler({ ownerPid: 10, readProcesses: async () => rows, sendSignal: (pid, value) => calls.push([pid, value]) })
  return { calls, signal }
}

test('MP-08/MP-10/MP-11 rejects missing, special, negative and noninteger child IDs before reading or signaling', async () => {
  const { signal, calls } = fixture([])
  for (const pid of [undefined, null, NaN, 0, 1, -1, -20, Infinity, 2.5, '20', 10]) {
    await assert.rejects(signal({ pid }, 'SIGTERM', { detached: true }), /unsafe/)
  }
  assert.deepEqual(calls, [])
})

test('MP-08/MP-10/MP-11 rejects foreign leader and mixed process group before any signal', async () => {
  for (const rows of [[member(20, 1)], [member(20, 10, 30)], [member(20, 10), member(21, 1)]]) {
    const { signal, calls } = fixture(rows)
    await assert.rejects(signal({ pid: 20 }, 'SIGTERM', { detached: true }), /ownership/)
    assert.deepEqual(calls, [])
  }
})

test('MP-08/MP-10/MP-11 signals verified descendants before leader with positive IDs only', async () => {
  const { signal, calls } = fixture([member(20, 10), member(21, 20), member(22, 21)])
  await signal({ pid: 20 }, 'SIGTERM', { detached: true })
  assert.deepEqual(calls, [[22, 'SIGTERM'], [21, 'SIGTERM'], [20, 'SIGTERM']])
})

test('MP-08/MP-10/MP-11 retains owned orphan identity for escalation and rejects PID reuse', async () => {
  let rows = [member(20, 10), member(21, 20)]
  const calls = [], child = { pid: 20 }
  const signal = createOwnedProcessSignaler({ ownerPid: 10, readProcesses: async () => rows, sendSignal: (pid, value) => calls.push([pid, value]) })
  await signal(child, 'SIGTERM', { detached: true })
  rows = [member(21, 1)]
  await signal(child, 'SIGKILL', { detached: true })
  assert.deepEqual(calls.at(-1), [21, 'SIGKILL'])
  rows = [member(21, 1, 20, '200')]
  await assert.rejects(signal(child, 'SIGKILL', { detached: true }), /ownership/)
  assert.equal(calls.length, 3)
})

test('MP-08/MP-10/MP-11 direct children never signal the caller process group', async () => {
  const { signal, calls } = fixture([member(20, 10, 10), member(21, 10, 10)])
  await signal({ pid: 20 }, 'SIGTERM')
  assert.deepEqual(calls, [[20, 'SIGTERM']])
})

test('MP-08/MP-10/MP-11 rechecks process start identity immediately before signaling', async () => {
  let reads = 0
  const calls = []
  const signal = createOwnedProcessSignaler({ ownerPid: 10, readProcesses: async () => [member(20, 10, 20, ++reads === 1 ? '100' : '200')], sendSignal: pid => calls.push(pid) })
  await assert.rejects(signal({ pid: 20 }, 'SIGTERM', { detached: true }), /ownership/)
  assert.deepEqual(calls, [])
})

test('MP-08/MP-10/MP-11 live detached child cleanup uses its owned tree', async () => {
  const child = spawn(process.execPath, ['-e', 'console.log("ready"); setInterval(() => {}, 1000)'], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] })
  const signal = createOwnedProcessSignaler()
  const exit = once(child, 'exit')
  await once(child.stdout, 'data')
  try {
    await signal(child, 'SIGTERM', { detached: true })
    assert.deepEqual(await exit, [null, 'SIGTERM'])
    await signal(child, 'SIGKILL', { detached: true })
  } finally {
    await signal(child, 'SIGKILL', { detached: true })
  }
})

test('MP-08/MP-10/MP-11 cleanup awaits SIGKILL acknowledgement after a resisted SIGTERM', async () => {
  const child = spawn(process.execPath, ['-e', 'process.on("SIGTERM",()=>{}); console.log("ready"); setInterval(()=>{},1000)'], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] })
  const exit = once(child, 'exit')
  await once(child.stdout, 'data')
  try {
    assert.equal(await stopOwnedProcess(child, { detached: true, graceMs: 50 }), true)
    assert.equal(child.signalCode, 'SIGKILL')
    assert.deepEqual(await exit, [null, 'SIGKILL'])
  } finally {
    await stopOwnedProcess(child, { detached: true, graceMs: 50 })
  }
})

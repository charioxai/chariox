// MP-08/MP-10/MP-11: no unsafe or foreign group may reach a signal API.
import test from 'node:test'
import assert from 'node:assert/strict'
import { createOwnedDrillProcessGroup } from './drill-owned-process-group.mjs'
const root = { pid: 21000, ppid: process.pid, pgid: 21000, started: 'root' }
const member = { pid: 21001, ppid: 21000, pgid: 21000, started: 'member' }
function fixture(initial = [root, member]) {
  let rows = initial
  const calls = []
  const child = { pid: root.pid, exitCode: null, signalCode: null }
  const group = createOwnedDrillProcessGroup(child, { snapshot: () => rows, signal: (...args) => calls.push(args) })
  return { child, group, calls, set: next => { rows = next } }
}
test('MP-08/MP-10/MP-11 reject invalid group IDs before inspecting or signaling', () => {
  for (const pid of [0, 1, -1, -20, undefined, null, NaN, Infinity, 2.5, '20']) {
    assert.throws(() => createOwnedDrillProcessGroup({ pid }, { snapshot: () => assert.fail('unsafe snapshot') }), /unsafe.*PID/)
  }
})
test('MP-08/MP-10/MP-11 signal only a verified group of owned descendants', () => {
  const f = fixture()
  assert.equal(f.group.signal('SIGTERM'), true)
  assert.deepEqual(f.calls, [[-root.pid, 'SIGTERM']])
  f.child.signalCode = 'SIGTERM'
  f.set([{ ...member, ppid: 1 }])
  assert.equal(f.group.signal('SIGKILL'), true)
  assert.deepEqual(f.calls.at(-1), [-root.pid, 'SIGKILL'])
  f.set([])
  assert.equal(f.group.signal('SIGKILL'), false)
})
test('MP-08/MP-10/MP-11 foreign member or PID reuse prevents any group signal', () => {
  for (const foreign of [{ pid: 21002, ppid: 1, pgid: root.pid, started: 'foreign' }, { ...member, started: 'reused' }]) {
    const f = fixture()
    f.set([root, foreign])
    assert.throws(() => f.group.signal('SIGTERM'), /foreign|reused/)
    assert.deepEqual(f.calls, [])
  }
})
test('MP-08/MP-10/MP-11 reject a group whose root is not this spawned child', () => {
  for (const changed of [{ ...root, ppid: 1 }, { ...root, pgid: 20999 }]) {
    assert.throws(() => fixture([changed]), /owned.*root/)
  }
})

test('MP-08/MP-10/MP-11 actual detached drill group and descendant settle', async () => {
  const { spawn } = await import('node:child_process')
  const { setTimeout: sleep } = await import('node:timers/promises')
  const { signalOwnedDrillChild } = await import('./drill-owned-child-signal.mjs')
  const source = `const {spawn}=require('node:child_process');spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});console.log('ready');setInterval(()=>{},1000)`
  const child = spawn(process.execPath, ['-e', source], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] })
  let group
  try {
    await Promise.race([
      new Promise((resolve, reject) => { child.stdout.once('data', resolve); child.once('error', reject) }),
      sleep(3000).then(() => { throw new Error('owned group startup timeout') }),
    ])
    group = createOwnedDrillProcessGroup(child)
    assert.equal(group.exists(), true)
    assert.equal(group.signal('SIGTERM'), true)
    for (let i = 0; i < 100 && group.exists(); i++) await sleep(10)
    assert.equal(child.exitCode !== null || child.signalCode !== null, true)
    // A reparented zombie is already stopped; the host's init may reap later.
    if (group.exists()) group.signal('SIGKILL')
  } finally {
    if (group?.exists()) group.signal('SIGKILL')
    if (child.exitCode === null && child.signalCode === null) signalOwnedDrillChild(child, 'SIGKILL')
  }
})

test('MP-08/MP-10/MP-11 snapshot failure or reused root fails closed', () => {
  const f = fixture()
  f.set([{ ...root, started: 'reused-root' }, member])
  assert.throws(() => f.group.signal('SIGTERM'), /reused/)
  assert.deepEqual(f.calls, [])
  let inspecting = false
  const calls = []
  const group = createOwnedDrillProcessGroup({ pid: root.pid }, {
    snapshot: () => { if (inspecting) throw new Error('snapshot unavailable'); return [root] },
    signal: (...args) => calls.push(args),
  })
  inspecting = true
  assert.throws(() => group.signal('SIGKILL'), /snapshot unavailable/)
  assert.deepEqual(calls, [])
})

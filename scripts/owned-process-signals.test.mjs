// MP-11 signal class enforcement: reject dangerous and unowned targets before I/O.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { once } from 'node:events'
import { createOwnedSignalGuard, spawnOwned, signalOwnedProcessGroup } from '../apps/kernel/slice-linux-docker/owned-process-signals.mjs'

const row = (pid, ppid = 9, pgid = 50, start = `start-${pid}`) => ({ pid, ppid, pgid, start })
function fixture() {
  let rows = [row(50)]
  const sent = []
  const guard = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  const handle = guard.record(50)
  return { guard, handle, sent, set: value => { rows = value } }
}
for (const id of [0, 1, -1, -2, undefined, NaN, Infinity, 2.5, '50', null]) {
  test(`MP-11 refuses invalid PID/group ${String(id)}`, () => {
    const { guard, sent } = fixture()
    assert.throws(() => guard.group(id, 'SIGTERM'), /invalid/)
    assert.throws(() => guard.pid(id, 'SIGTERM'), /invalid/)
    assert.throws(() => guard.record(id), /invalid/)
    assert.deepEqual(sent, [])
  })
}
test('MP-11 verifies all live group members and rejects a foreign member', () => {
  const f = fixture()
  f.set([row(50), row(51, 50)])
  f.guard.group(f.handle, 'SIGTERM')
  assert.deepEqual(f.sent, [[-50, 'SIGTERM']])
  f.set([row(50), row(51, 50), row(90, 9)])
  assert.throws(() => f.guard.group(f.handle, 'SIGKILL'), /unowned/)
  assert.equal(f.sent.length, 1)
})
test('MP-11 refuses a reused PID/group and unknown members after leader exit', () => {
  for (const rows of [[row(50, 9, 50, 'reused')], [row(99)]]) {
    const f = fixture(); f.set(rows)
    assert.throws(() => f.guard.group(f.handle, 'SIGTERM'), /unowned/)
    if (rows.some(row => row.pid === 50)) assert.throws(() => f.guard.pid(f.handle, 'SIGTERM'), /unowned/)
    else assert.equal(f.guard.pid(f.handle, 'SIGTERM'), false)
    assert.deepEqual(f.sent, [])
  }
})
test('MP-11 permits previously recorded survivors after leader exit, never a reused member', () => {
  const f = fixture(); f.set([row(50), row(51, 50)]); f.guard.refresh()
  f.set([row(51, 1)]); f.guard.group(f.handle, 'SIGKILL')
  assert.deepEqual(f.sent, [[-50, 'SIGKILL']])
  f.set([row(51, 1, 50, 'reused')]); assert.throws(() => f.guard.group(f.handle, 'SIGKILL'), /unowned/)
})
test('MP-11 refuses missing identity/snapshot failure and treats an absent group as settled', () => {
  const f = fixture(); f.set([]); assert.equal(f.guard.group(f.handle, 'SIGTERM'), false)
  assert.equal(f.sent.length, 0)
  assert.throws(() => createOwnedSignalGuard({ snapshot: () => { throw Error('unavailable') } }).record(50), /unavailable/)
  assert.throws(() => createOwnedSignalGuard({ snapshot: () => [{...row(50), start: ''}] }).record(50), /identity/)
})
test('MP-11 exclusive live session proves reparented members without granting reused sessions', () => {
  let rows = [{...row(50), sid: 50}]
  const sent = []
  const g = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  const handle = g.record(50)
  rows.push({...row(51, 1), sid: 50})
  g.group(handle, 'SIGTERM')
  rows = [{...row(50, 9, 50, 'new-root'), sid: 50}, {...row(52, 1), sid: 50}]
  assert.throws(() => g.group(handle, 'SIGKILL'), /unowned/)
  assert.equal(sent.length, 1)
})
test('MP-11 BSD session tokens require an explicitly recorded live detached leader', () => {
  let rows = [{...row(50), session: '0xabc'}]
  const sent = []
  const g = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  const handle = g.record(50, { sessionLeader: true })
  rows.push({...row(51, 1), session: '0xabc'})
  g.group(handle, 'SIGTERM')
  rows = [{...row(50, 9, 50, 'new-root'), session: '0xabc'}, {...row(52, 1), session: '0xabc'}]
  assert.throws(() => g.group(handle, 'SIGKILL'), /unowned/)
  assert.equal(sent.length, 1)
})
test('MP-11 live detached child cleanup uses the shared guard', async () => {
  const child = spawnOwned(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { detached: true, stdio: 'ignore' })
  const exit = once(child, 'exit')
  assert.equal(signalOwnedProcessGroup(child, 'SIGTERM'), true)
  await exit
  assert.equal(signalOwnedProcessGroup(child, 'SIGKILL'), false)
})

// MP-11 #868: a completed launch must not reserve its numeric PID forever.
test('MP-11 fresh launch reuses a retired PID but stale handles cannot signal it', () => {
  let rows = [row(50)]
  const sent = []
  const g = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  const old = g.record(50)
  rows = []
  g.refresh()
  rows = [row(50, 9, 50, 'generation-two')]
  const fresh = g.record(50, { freshLaunch: true })
  assert.throws(() => g.group(old, 'SIGTERM'), /unowned|stale|generation/)
  assert.throws(() => g.pid(old, 'SIGTERM'), /unowned|stale|generation/)
  assert.deepEqual(sent, [])
  assert.equal(g.group(fresh, 'SIGTERM'), true)
  rows.push(row(51, 50))
  g.refresh()
  rows = [row(50, 9, 50, 'generation-two'), row(51, 50, 50, 'new-descendant')]
  assert.equal(g.group(fresh, 'SIGTERM'), true)
  assert.equal(sent.length, 2)
})

test('MP-11 closing a leader retains verified survivors without owning a replacement', () => {
  const f = fixture()
  const old = f.guard.record(50)
  f.set([row(50), row(51, 50)]); f.guard.refresh()
  f.set([row(51, 1)]); f.guard.retire(old)
  assert.equal(f.guard.group(old, 'SIGTERM'), true)
  f.set([row(50, 9, 50, 'new-launch'), row(51, 1)])
  const fresh = f.guard.record(50, { freshLaunch: true })
  assert.throws(() => f.guard.group(old, 'SIGKILL'), /unowned/)
  assert.throws(() => f.guard.group(fresh, 'SIGKILL'), /unowned/)
  f.set([row(50, 9, 50, 'new-launch')])
  assert.equal(f.guard.group(fresh, 'SIGTERM'), true)
})

// MP-11 #868 round 2: the production cleanup shape must not resolve a retained
// numeric child.pid through the replacement generation's latest registration.
test('MP-11 retained numeric production cleanup target never selects a reused launch', () => {
  const f = fixture()
  const child = { pid: 50 }
  const old = f.guard.record(child.pid)
  f.set([]); f.guard.retire(old)
  f.set([row(50, 9, 50, 'replacement')])
  const replacement = f.guard.record(50, { freshLaunch: true })
  assert.throws(() => f.guard.group(child.pid, 'SIGTERM'), /generation|handle/)
  assert.deepEqual(f.sent, [])
  assert.equal(f.guard.group(replacement, 'SIGTERM'), true)
})

test('MP-11 subgroup and descendant PID handles retain their launch generation after leader exit', () => {
  const f = fixture()
  f.set([row(50), row(51, 50, 51)])
  const subgroup = f.guard.groupHandle(f.handle, 51)
  const descendant = f.guard.pidHandle(f.handle, 51)
  f.set([row(51, 1, 51)])
  assert.equal(f.guard.group(subgroup, 'SIGTERM'), true)
  assert.equal(f.guard.pid(descendant, 'SIGTERM'), true)
  f.set([row(51, 1, 51), row(90, 9, 51)])
  assert.throws(() => f.guard.group(subgroup, 'SIGKILL'), /unowned/)
  f.set([row(51, 9, 51, 'replacement')])
  const replacement = f.guard.record(51, { freshLaunch: true })
  assert.throws(() => f.guard.group(subgroup, 'SIGKILL'), /unowned/)
  assert.throws(() => f.guard.pid(descendant, 'SIGKILL'), /unowned/)
  assert.equal(f.sent.length, 2)
  assert.equal(f.guard.group(replacement, 'SIGTERM'), true)
})

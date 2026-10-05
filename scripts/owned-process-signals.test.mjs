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
  guard.record(50)
  return { guard, sent, set: value => { rows = value } }
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
  f.guard.group(50, 'SIGTERM')
  assert.deepEqual(f.sent, [[-50, 'SIGTERM']])
  f.set([row(50), row(51, 50), row(90, 9)])
  assert.throws(() => f.guard.group(50, 'SIGKILL'), /unowned/)
  assert.equal(f.sent.length, 1)
})
test('MP-11 refuses a reused PID/group and unknown members after leader exit', () => {
  for (const rows of [[row(50, 9, 50, 'reused')], [row(99)]]) {
    const f = fixture(); f.set(rows)
    assert.throws(() => f.guard.group(50, 'SIGTERM'), /unowned/)
    if (rows.some(row => row.pid === 50)) assert.throws(() => f.guard.pid(50, 'SIGTERM'), /unowned/)
    else assert.equal(f.guard.pid(50, 'SIGTERM'), false)
    assert.deepEqual(f.sent, [])
  }
})
test('MP-11 permits previously recorded survivors after leader exit, never a reused member', () => {
  const f = fixture(); f.set([row(50), row(51, 50)]); f.guard.refresh()
  f.set([row(51, 1)]); f.guard.group(50, 'SIGKILL')
  assert.deepEqual(f.sent, [[-50, 'SIGKILL']])
  f.set([row(51, 1, 50, 'reused')]); assert.throws(() => f.guard.group(50, 'SIGKILL'), /unowned/)
})
test('MP-11 refuses missing identity/snapshot failure and treats an absent group as settled', () => {
  const f = fixture(); f.set([]); assert.equal(f.guard.group(50, 'SIGTERM'), false)
  assert.equal(f.sent.length, 0)
  assert.throws(() => createOwnedSignalGuard({ snapshot: () => { throw Error('unavailable') } }).record(50), /unavailable/)
  assert.throws(() => createOwnedSignalGuard({ snapshot: () => [{...row(50), start: ''}] }).record(50), /identity/)
})
test('MP-11 exclusive live session proves reparented members without granting reused sessions', () => {
  let rows = [{...row(50), sid: 50}]
  const sent = []
  const g = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  g.record(50)
  rows.push({...row(51, 1), sid: 50})
  g.group(50, 'SIGTERM')
  rows = [{...row(50, 9, 50, 'new-root'), sid: 50}, {...row(52, 1), sid: 50}]
  assert.throws(() => g.group(50, 'SIGKILL'), /unowned/)
  assert.equal(sent.length, 1)
})
test('MP-11 BSD session tokens require an explicitly recorded live detached leader', () => {
  let rows = [{...row(50), session: '0xabc'}]
  const sent = []
  const g = createOwnedSignalGuard({ snapshot: () => rows, send: (...args) => sent.push(args) })
  g.record(50, { sessionLeader: true })
  rows.push({...row(51, 1), session: '0xabc'})
  g.group(50, 'SIGTERM')
  rows = [{...row(50, 9, 50, 'new-root'), session: '0xabc'}, {...row(52, 1), session: '0xabc'}]
  assert.throws(() => g.group(50, 'SIGKILL'), /unowned/)
  assert.equal(sent.length, 1)
})
test('MP-11 live detached child cleanup uses the shared guard', async () => {
  const child = spawnOwned(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { detached: true, stdio: 'ignore' })
  const exit = once(child, 'exit')
  assert.equal(signalOwnedProcessGroup(child, 'SIGTERM'), true)
  await exit
  assert.equal(signalOwnedProcessGroup(child, 'SIGKILL'), false)
})

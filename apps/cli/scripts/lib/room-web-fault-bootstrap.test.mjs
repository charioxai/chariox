// MP-08 / MP-10: retain HTTP failure while refusing malformed successful bootstrap.
import assert from 'node:assert/strict'
import test from 'node:test'
import { observeRoomFaultBootstrap } from './room-web-fault-bootstrap.mjs'
test('bootstrap outage reaches UI without an online assertion; successful responses remain fenced', () => {
  let count = 0
  const validate = body => { count++; assert.equal(body.target?.daemonId, 'owned') }
  assert.equal(observeRoomFaultBootstrap({ error: 'offline' }, { httpStatus: 503 }, validate), false)
  assert.equal(count, 0)
  assert.equal(observeRoomFaultBootstrap({ target: { daemonId: 'owned' } }, { httpStatus: 200 }, validate), true)
  assert.throws(() => observeRoomFaultBootstrap({ target: { daemonId: 'foreign' } }, { httpStatus: 200 }, validate))
  assert.throws(() => observeRoomFaultBootstrap({}, { httpStatus: 200 }, validate))
  assert.throws(() => observeRoomFaultBootstrap({}, {}, validate))
})

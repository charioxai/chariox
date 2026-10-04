// MP-08/MP-10/MP-11: dangerous cleanup targets must never reach a signal API.
import assert from 'node:assert/strict'
import test from 'node:test'
import { signalOwnedDrillChild } from './drill-owned-child-signal.mjs'

test('MP-08/MP-10/MP-11 cleanup rejects invalid and special PIDs before signaling', () => {
  for (const pid of [0, 1, -1, -20, undefined, null, NaN, Infinity, 2.5, '20']) {
    let calls = 0
    const child = { pid, exitCode: null, signalCode: null, kill() { calls++; return true } }
    assert.throws(() => signalOwnedDrillChild(child, 'SIGTERM'), /unsafe.*PID/, String(pid))
    assert.equal(calls, 0)
  }
  assert.throws(() => signalOwnedDrillChild(undefined, 'SIGTERM'), /unsafe.*PID/)
})

test('MP-08/MP-10/MP-11 cleanup forwards an owned live child signal', () => {
  const calls = []
  const child = { pid: 12345, exitCode: null, signalCode: null, kill(signal) { calls.push(signal); return true } }
  assert.equal(signalOwnedDrillChild(child, 'SIGTERM'), true)
  assert.deepEqual(calls, ['SIGTERM'])
})

test('MP-08/MP-10/MP-11 cleanup preserves already settled child processes', () => {
  for (const state of [{ exitCode: 0, signalCode: null }, { exitCode: null, signalCode: 'SIGTERM' }]) {
    const child = { pid: 12345, ...state, kill() { assert.fail('settled child signaled') } }
    assert.equal(signalOwnedDrillChild(child, 'SIGKILL'), false)
  }
})

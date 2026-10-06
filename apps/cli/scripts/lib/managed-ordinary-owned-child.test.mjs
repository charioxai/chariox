import assert from "node:assert/strict"
import test from "node:test"
import { signalOwnedChild } from "./managed-ordinary-owned-child.mjs"

test("MP-10 signal guard rejects reserved, group, invalid and exited process identities", () => {
  for (const pid of [0, 1, -1, -42, undefined, NaN, Infinity, "42", 1.5]) {
    assert.equal(signalOwnedChild({ pid, exitCode: null, signalCode: null,
      kill: () => assert.fail("unsafe signal") }), false)
  }
  for (const state of [{ exitCode: 0, signalCode: null }, { exitCode: null, signalCode: "SIGTERM" }]) {
    assert.equal(signalOwnedChild({ pid: 42, ...state, kill: () => assert.fail("settled child signal") }), false)
  }
  const signals = []
  assert.equal(signalOwnedChild({ pid: 42, exitCode: null, signalCode: null,
    kill: signal => { signals.push(signal); return true } }), true)
  assert.deepEqual(signals, ["SIGTERM"])
})

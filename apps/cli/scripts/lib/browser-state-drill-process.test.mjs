import assert from "node:assert/strict"
import test from "node:test"
import { signalBrowserStateChild } from "./browser-state-drill-process.mjs"

test("MP-10: cleanup rejects missing, reserved, group, and invalid process IDs", () => {
  for (const pid of [undefined, null, NaN, 0, 1, -1, -42, 1.5, Infinity, "42"]) {
    let called = false
    assert.equal(signalBrowserStateChild({ pid, kill() { called = true } }, "SIGTERM"), false)
    assert.equal(called, false, `unsafe pid ${String(pid)} reached kill`)
  }
  assert.equal(signalBrowserStateChild(undefined, "SIGTERM"), false)
})

test("MP-10: cleanup signals only a running owned child, including escalation", () => {
  const signals = []
  const child = { pid: 42, exitCode: null, signalCode: null, kill(signal) { signals.push(signal); return true } }
  assert.equal(signalBrowserStateChild(child, "SIGTERM"), true)
  assert.equal(signalBrowserStateChild(child, "SIGKILL"), true)
  child.signalCode = "SIGTERM"
  assert.equal(signalBrowserStateChild(child, "SIGKILL"), false)
  child.signalCode = null
  child.exitCode = 0
  assert.equal(signalBrowserStateChild(child, "SIGTERM"), false)
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"])
})

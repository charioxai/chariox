// MP-08/MP-10/MP-11: mocks record signal intent; no real process is signalled.
import assert from "node:assert/strict"
import test from "node:test"
import type { ChildProcess } from "node:child_process"
import { signalNativeChild } from "./native-child-signals.js"
for (const pid of [undefined, 0, 1, -1, -22, NaN, Infinity, 1.5]) test(`native cleanup rejects invalid PID ${pid}`, () => {
  let calls = 0
  const child = { pid, exitCode: null, signalCode: null, kill: () => { calls++; return true } } as Pick<ChildProcess, "pid" | "exitCode" | "signalCode" | "kill">
  assert.equal(signalNativeChild(child, "SIGTERM"), false)
  assert.equal(calls, 0)
})
test("native cleanup preserves the requested signal for a live owned child", () => {
  const signals: string[] = []
  const child = { pid: 42, exitCode: null, signalCode: null, kill: (signal: string) => { signals.push(signal); return true } } as Pick<ChildProcess, "pid" | "exitCode" | "signalCode" | "kill">
  assert.equal(signalNativeChild(child, "SIGTERM"), true)
  assert.equal(signalNativeChild(child, "SIGKILL"), true)
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"])
})
for (const [exitCode, signalCode] of [[0, null], [null, "SIGTERM"]] as const) test("native cleanup skips an exited child", () => {
  const child = {pid: 42, exitCode, signalCode, kill: () => { throw new Error("must not signal an exited child") }} as Pick<ChildProcess, "pid" | "exitCode" | "signalCode" | "kill">
  assert.equal(signalNativeChild(child, "SIGKILL"), false)
})

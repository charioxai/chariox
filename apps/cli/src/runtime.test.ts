import test from "node:test"
import assert from "node:assert/strict"

import { LocalIpcError } from "./ipc.js"
import {
  DEFAULT_CONNECTED_STATUS,
  MAX_TRANSIENT_POLL_FAILURES,
  describeCliError,
  getExitCleanupDecision,
  getPollRecoveryDecision,
  shouldEndSessionOnCliExit,
} from "./runtime.js"

test("describeCliError prefers structured error messages", () => {
  assert.equal(
    describeCliError(new LocalIpcError("connect local socket", "timed out")),
    "local transport `connect local socket` failed: timed out",
  )
  assert.equal(describeCliError(new Error("boom")), "boom")
  assert.equal(describeCliError("plain"), "plain")
  assert.equal(DEFAULT_CONNECTED_STATUS, "")
})

test("poll recovery retries transient IPC failures with backoff", () => {
  const failure = new LocalIpcError("handle local response", "timed out")

  const first = getPollRecoveryDecision("polling terminal output", failure, 1)
  assert.equal(first.retry, true)
  assert.equal(first.delayMs, 250)
  assert.match(first.message, /retrying \(1\/4\)/)

  const second = getPollRecoveryDecision("polling terminal output", failure, 2)
  assert.equal(second.retry, true)
  assert.equal(second.delayMs, 500)

  const terminal = getPollRecoveryDecision(
    "polling terminal output",
    failure,
    MAX_TRANSIENT_POLL_FAILURES,
  )
  assert.equal(terminal.retry, false)
  assert.equal(terminal.delayMs, 0)
  assert.match(terminal.message, /Lost connection while polling terminal output/)
})

test("poll recovery does not retry non transport errors", () => {
  const decision = getPollRecoveryDecision(
    "polling notices",
    new Error("unexpected response variant"),
    1,
  )

  assert.equal(decision.retry, false)
  assert.equal(decision.delayMs, 0)
  assert.equal(decision.message, "unexpected response variant")
})

test("exit cleanup requires a second attempt before forcing exit", () => {
  const first = getExitCleanupDecision(
    new LocalIpcError("handle local response", "timed out"),
    false,
  )
  assert.equal(first.exit, false)
  assert.equal(first.exitCode, 1)
  assert.match(first.message, /Run \/exit or press Ctrl\+C again to force quit/)

  const second = getExitCleanupDecision(
    new LocalIpcError("handle local response", "timed out"),
    true,
  )
  assert.equal(second.exit, true)
  assert.equal(second.exitCode, 1)
  assert.match(second.message, /Forcing exit/)
})

test("cli exit detaches instead of ending the session", () => {
  assert.equal(shouldEndSessionOnCliExit(true, 1), false)
  assert.equal(shouldEndSessionOnCliExit(true, 3), false)
  assert.equal(shouldEndSessionOnCliExit(false, 1), false)
})

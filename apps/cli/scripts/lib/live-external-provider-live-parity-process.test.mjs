import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"

import { closeWithTimeout } from "./live-external-provider-live-parity-process.mjs"

test("closeWithTimeout cancels its timeout after a graceful close", async () => {
  const warnings = []
  const originalWarn = console.warn
  console.warn = (message) => warnings.push(String(message))
  try {
    await closeWithTimeout({ close: async () => {} }, "browser", 10)
    await sleep(3_100)
  } finally {
    console.warn = originalWarn
  }

  assert.deepEqual(warnings, [])
})

test("closeWithTimeout kills a process-backed target after a real timeout", async () => {
  const warnings = []
  const signals = []
  const originalWarn = console.warn
  console.warn = (message) => warnings.push(String(message))
  try {
    await closeWithTimeout({
      close: () => new Promise(() => undefined),
      process: () => ({ kill: (signal) => signals.push(signal) }),
    }, "browser", 10)
  } finally {
    console.warn = originalWarn
  }

  assert.deepEqual(warnings, ["browser close timed out"])
  assert.deepEqual(signals, ["SIGKILL"])
})

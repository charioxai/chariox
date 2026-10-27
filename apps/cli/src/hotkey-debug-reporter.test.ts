import { strict as assert } from "node:assert"
import test from "node:test"

import { createHotkeyDebugReporter } from "./hotkey-debug-reporter.js"

test("hotkey debug reporter logs without flashing when debug footer output is disabled", () => {
  const logs: Array<{ message: string, fields: Record<string, unknown> | undefined }> = []
  const flashes: Array<{ message: string, tone: string }> = []
  const reporter = createHotkeyDebugReporter({
    debugLogsEnabled: false,
    logDebug: (message, fields) => logs.push({ message, fields }),
    flashFooter: (message, tone) => flashes.push({ message, tone }),
  })

  reporter.report("focus changed")

  assert.deepEqual(logs, [
    { message: "hotkeys footer debug", fields: { detail: "focus changed" } },
  ])
  assert.deepEqual(flashes, [])
})

test("hotkey debug reporter flashes when debug footer output is enabled", () => {
  const flashes: Array<{ message: string, tone: string }> = []
  const reporter = createHotkeyDebugReporter({
    debugLogsEnabled: true,
    logDebug: () => {},
    flashFooter: (message, tone) => flashes.push({ message, tone }),
  })

  reporter.report("saved focus")

  assert.deepEqual(flashes, [
    { message: "[hotkeys] saved focus", tone: "info" },
  ])
})

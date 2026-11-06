import assert from "node:assert/strict"
import test from "node:test"

import { createCliProcessLifecycleController } from "./cli-process-lifecycle-controller.js"

test("CLI process lifecycle starts and stops owned process resources", () => {
  const calls: string[] = []
  const handleSigint = () => {}

  const controller = createCliProcessLifecycleController({
    handleSigint,
    startAutomationServer: () => calls.push("start-server"),
    stopAutomationServer: () => calls.push("stop-server"),
    onSigint: (handler) => calls.push(handler === handleSigint ? "on-sigint" : "on-other"),
    offSigint: (handler) => calls.push(handler === handleSigint ? "off-sigint" : "off-other"),
    clearTerminalOutputRecordTimer: () => calls.push("clear-terminal-timer"),
  })

  controller.start()
  controller.stop()

  assert.deepEqual(calls, [
    "start-server",
    "on-sigint",
    "off-sigint",
    "stop-server",
    "clear-terminal-timer",
  ])
})

test("CLI process lifecycle start and stop are idempotent", () => {
  const calls: string[] = []
  const controller = createCliProcessLifecycleController({
    handleSigint: () => {},
    startAutomationServer: () => calls.push("start-server"),
    stopAutomationServer: () => calls.push("stop-server"),
    onSigint: () => calls.push("on-sigint"),
    offSigint: () => calls.push("off-sigint"),
    clearTerminalOutputRecordTimer: () => calls.push("clear-terminal-timer"),
  })

  controller.start()
  controller.start()
  controller.stop()
  controller.stop()

  assert.deepEqual(calls, [
    "start-server",
    "on-sigint",
    "off-sigint",
    "stop-server",
    "clear-terminal-timer",
  ])
})

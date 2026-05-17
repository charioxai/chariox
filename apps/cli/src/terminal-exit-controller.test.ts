import assert from "node:assert/strict"
import test from "node:test"

import { createTerminalExitController } from "./terminal-exit-controller.js"

test("restoreAndExit disables terminal modes, destroys the renderer, sleeps, and exits", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => {
        events.push("kitty")
      },
      disableStdoutInterception: () => {
        events.push("stdout")
      },
      destroy: () => {
        events.push("destroy")
      },
    },
    sleep: async (delayMs) => {
      assert.equal(delayMs, 25)
      events.push("sleep")
    },
    exitProcess: (exitCode) => {
      assert.equal(exitCode, 7)
      events.push("exit")
      throw new Error("process exit")
    },
    onRendererDestroyFailed: () => {},
  })

  await assert.rejects(() => controller.restoreAndExit(7), /process exit/)

  assert.deepEqual(events, ["kitty", "stdout", "destroy", "sleep", "exit"])
})

test("restoreAndExit tolerates terminal mode cleanup failures", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => {
        throw new Error("kitty failed")
      },
      disableStdoutInterception: () => {
        throw new Error("stdout failed")
      },
      destroy: () => {
        events.push("destroy")
      },
    },
    sleep: async () => {
      events.push("sleep")
    },
    exitProcess: () => {
      events.push("exit")
      throw new Error("process exit")
    },
    onRendererDestroyFailed: () => {},
  })

  await assert.rejects(() => controller.restoreAndExit(0), /process exit/)

  assert.deepEqual(events, ["destroy", "sleep", "exit"])
})

test("restoreAndExit skips destroyed renderers and reports destroy failures", async () => {
  const failures: string[] = []
  const skippedDestroyController = createTerminalExitController({
    renderer: {
      isDestroyed: true,
      disableKittyKeyboard: () => {},
      disableStdoutInterception: () => {},
      destroy: () => {
        throw new Error("unexpected destroy")
      },
    },
    sleep: async () => {},
    exitProcess: () => {
      throw new Error("process exit")
    },
    onRendererDestroyFailed: (error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
  })

  await assert.rejects(() => skippedDestroyController.restoreAndExit(0), /process exit/)
  assert.equal(failures.length, 0)

  const failedDestroyController = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => {},
      disableStdoutInterception: () => {},
      destroy: () => {
        throw new Error("destroy failed")
      },
    },
    sleep: async () => {},
    exitProcess: () => {
      throw new Error("process exit")
    },
    onRendererDestroyFailed: (error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
  })

  await assert.rejects(() => failedDestroyController.restoreAndExit(0), /process exit/)
  assert.deepEqual(failures, ["destroy failed"])
})

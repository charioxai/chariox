import assert from "node:assert/strict"
import test from "node:test"

import { createTerminalExitController, TERMINAL_MODE_RESTORE_SEQUENCE } from "./terminal-exit-controller.js"

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

test("restoreAndExit writes the terminal mode resets itself when the renderer cannot restore them", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => { events.push("kitty") },
      disableStdoutInterception: () => { events.push("stdout") },
      destroy: () => {
        events.push("destroy")
        throw new Error("setRawMode failed with errno: 5")
      },
    },
    sleep: async () => { events.push("sleep") },
    exitProcess: () => {
      events.push("exit")
      throw new Error("process exit")
    },
    onRendererDestroyFailed: () => { events.push("failed") },
    restoreTerminalModes: () => { events.push("modes") },
  })

  await assert.rejects(() => controller.restoreAndExit(1), /process exit/)

  assert.deepEqual(events, ["kitty", "stdout", "destroy", "failed", "modes", "sleep", "exit"])
})

test("restoreAndExit leaves a successful renderer restore alone", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => {},
      disableStdoutInterception: () => {},
      destroy: () => { events.push("destroy") },
    },
    sleep: async () => {},
    exitProcess: () => { throw new Error("process exit") },
    onRendererDestroyFailed: () => { events.push("failed") },
    restoreTerminalModes: () => { events.push("modes") },
  })

  await assert.rejects(() => controller.restoreAndExit(0), /process exit/)

  assert.deepEqual(events, ["destroy"])
})

test("a failed mode write cannot keep the process from exiting", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: {
      isDestroyed: false,
      disableKittyKeyboard: () => {},
      disableStdoutInterception: () => {},
      destroy: () => { throw new Error("destroy failed") },
    },
    sleep: async () => {},
    exitProcess: () => {
      events.push("exit")
      throw new Error("process exit")
    },
    onRendererDestroyFailed: () => {},
    restoreTerminalModes: () => { throw new Error("EIO") },
  })

  await assert.rejects(() => controller.restoreAndExit(1), /process exit/)

  assert.deepEqual(events, ["exit"])
})

test("the fallback resets every mode the TUI enables, in an order that leaves the cursor visible", () => {
  for (const mode of ["1049", "1000", "1002", "1003", "1006", "1004", "2004", "2031", "2027", "2026"]) {
    assert.ok(TERMINAL_MODE_RESTORE_SEQUENCE.includes(`\u001b[?${mode}l`), `missing reset for mode ${mode}`)
  }
  assert.ok(TERMINAL_MODE_RESTORE_SEQUENCE.includes("\u001b[>4;0m"), "missing modifyOtherKeys reset")
  assert.ok(TERMINAL_MODE_RESTORE_SEQUENCE.endsWith("\u001b[?25h"), "cursor must be shown last")
  assert.ok(
    TERMINAL_MODE_RESTORE_SEQUENCE.indexOf("\u001b[?1049l") < TERMINAL_MODE_RESTORE_SEQUENCE.indexOf("\u001b[?25h"),
    "leave the alternate screen before showing the cursor",
  )
})

// The alternate screen discards anything written while it is active, so the
// exit reason must follow the restoration (renderer or fallback) and precede exit.
for (const destroyFails of [false, true]) {
  test(`the exit reason is written after restoration and before exit (renderer destroy ${destroyFails ? "fails" : "succeeds"})`, async () => {
    const events: string[] = []
    const controller = createTerminalExitController({
      renderer: {
        isDestroyed: false,
        disableKittyKeyboard: () => { events.push("kitty") },
        disableStdoutInterception: () => { events.push("stdout") },
        destroy: () => {
          events.push("destroy")
          if (destroyFails) throw new Error("destroy failed")
        },
      },
      sleep: async () => { events.push("flush") },
      exitProcess: () => {
        events.push("exit")
        throw new Error("process exit")
      },
      onRendererDestroyFailed: () => {},
      restoreTerminalModes: () => { events.push("modes") },
      writeMessage: (message) => { events.push(`message:${message}`) },
    })

    await assert.rejects(() => controller.restoreAndExit(1, "chariox: reason\n"), /process exit/)

    assert.deepEqual(events, [
      "kitty", "stdout", "destroy", ...(destroyFails ? ["modes"] : []), "message:chariox: reason\n", "flush", "exit",
    ])
  })
}

test("a failed message write cannot keep the process from exiting", async () => {
  const events: string[] = []
  const controller = createTerminalExitController({
    renderer: { isDestroyed: true, disableKittyKeyboard: () => {}, disableStdoutInterception: () => {}, destroy: () => {} },
    sleep: async () => {},
    exitProcess: () => {
      events.push("exit")
      throw new Error("process exit")
    },
    onRendererDestroyFailed: () => {},
    writeMessage: () => { throw new Error("EIO") },
  })

  await assert.rejects(() => controller.restoreAndExit(1, "chariox: reason\n"), /process exit/)

  assert.deepEqual(events, ["exit"])
})

test("restoreAndExit without a reason writes nothing", async () => {
  const controller = createTerminalExitController({
    renderer: { isDestroyed: true, disableKittyKeyboard: () => {}, disableStdoutInterception: () => {}, destroy: () => {} },
    sleep: async () => {},
    exitProcess: () => { throw new Error("process exit") },
    onRendererDestroyFailed: () => {},
    writeMessage: () => assert.fail("wrote a message without a reason"),
  })

  await assert.rejects(() => controller.restoreAndExit(0), /process exit/)
})

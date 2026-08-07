import assert from "node:assert/strict"
import test from "node:test"

import { handleWaitingRoomSlashCommand } from "./waiting-room-slash-command-policy.js"

test("waiting-room slash policy blocks session-scoped commands without bootstrapping", async () => {
  const harness = createHarness()

  assert.equal(await harness.handle("/agent focus 1"), true)

  assert.deepEqual(harness.flashes, [{ message: "start or join a session first", tone: "error" }])
  assert.equal(harness.clearedPrompts, 1)
  assert.equal(harness.clearedCommandCenters, 1)
})

test("waiting-room slash policy lets detached-safe commands use normal slash handlers", async () => {
  const harness = createHarness()

  assert.equal(await harness.handle("/provider codex"), false)
  assert.equal(await harness.handle("/session list"), false)
  assert.equal(await harness.handle("/mode plan"), false)
  assert.equal(await harness.handle("/permissions required"), false)
  assert.equal(await harness.handle("/view split"), false)

  assert.deepEqual(harness.flashes, [])
  assert.equal(harness.clearedPrompts, 0)
})

test("waiting-room slash policy handles waiting room and unknown commands", async () => {
  const harness = createHarness()

  assert.equal(await harness.handle("/waiting"), true)
  assert.equal(await harness.handle("/not-real"), true)

  assert.deepEqual(harness.flashes, [
    { message: "already in waiting room", tone: "info" },
    { message: "/not-real is not wired in the TUI yet", tone: "error" },
  ])
  assert.equal(harness.clearedPrompts, 1)
})

function createHarness() {
  const flashes: Array<{ message: string; tone: "info" | "error" }> = []
  let clearedPrompts = 0
  let clearedCommandCenters = 0
  return {
    flashes,
    get clearedPrompts() {
      return clearedPrompts
    },
    get clearedCommandCenters() {
      return clearedCommandCenters
    },
    handle(rawPrompt: string) {
      return handleWaitingRoomSlashCommand(rawPrompt, {
        clearCommandCenter: () => {
          clearedCommandCenters += 1
        },
        clearPromptText: () => {
          clearedPrompts += 1
        },
        flashFooter: (message, tone) => {
          flashes.push({ message, tone })
        },
      })
    },
  }
}

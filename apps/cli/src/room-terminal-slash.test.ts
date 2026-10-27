import assert from "node:assert/strict"
import test from "node:test"

import { TERMINAL_DETACH_BYTE } from "./room-terminal-attach.js"
import { FakeTerminalIo, FakeTerminalKernel, until } from "./room-terminal-fake-kernel.test-fixture.js"
import { handleRoomTerminalSlash, type RoomTerminalSlashDeps } from "./room-terminal-slash.js"

function harness(overrides: Partial<RoomTerminalSlashDeps> = {}) {
  const kernel = new FakeTerminalKernel()
  const events: string[] = []
  const deps: RoomTerminalSlashDeps = {
    isAttached: () => true,
    sessionId: () => "room",
    send: kernel.send,
    screen: { suspend: () => events.push("suspend"), resume: () => events.push("resume") },
    appendNotice: message => events.push(`notice: ${message}`),
    flashFooter: (message, tone) => events.push(`${tone}: ${message}`),
    ...overrides,
  }
  return { kernel, events, deps }
}

test("/room terminal list prints the Room's terminals without touching the screen", async () => {
  const { events, deps } = harness()
  await handleRoomTerminalSlash(deps, ["list"])
  assert.equal(events.length, 1)
  assert.match(events[0]!, /^notice: 1\. term-1/)
})

test("/room terminal needs an attached Room session and a valid command", async () => {
  const detached = harness({ isAttached: () => false })
  await handleRoomTerminalSlash(detached.deps, [])
  assert.deepEqual(detached.events, ["error: Attach to a Room session to use its terminals"])
  assert.equal(detached.kernel.requests.length, 0)
  const wrong = harness()
  await handleRoomTerminalSlash(wrong.deps, ["close"])
  assert.match(wrong.events[0]!, /^error: usage: \/room terminal /)
})

test("/room terminal hands the screen to the terminal and takes it back on detach", async () => {
  const { events, deps, kernel } = harness()
  const done = handleRoomTerminalSlash(deps, [])
  await until(() => events.includes("suspend"), "the screen handover")
  await until(() => kernel.requests.some(request => request.operation === "snapshot"), "the attach")
  assert.deepEqual(events, ["suspend"])
  process.stdin.emit("data", Buffer.from([TERMINAL_DETACH_BYTE]))
  await done
  assert.equal(events[1], "resume")
  assert.match(events[2]!, /^info: Detached from terminal term-1; it keeps running/)
})

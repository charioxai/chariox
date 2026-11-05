import assert from "node:assert/strict"
import test from "node:test"

import { createTerminalResizeController } from "./terminal-resize-controller.js"

test("terminal resize controller resizes only attached sessions", () => {
  const resizedSessions: string[] = []
  let attached = false
  let sessionId = "session-a"
  const controller = createTerminalResizeController({
    isAttached: () => attached,
    sessionId: () => sessionId,
    resizeSession: (id) => {
      resizedSessions.push(id)
    },
    rebuildWaitingRoom: () => {},
  })

  assert.equal(controller.handleResize(), false)
  assert.deepEqual(resizedSessions, [])

  attached = true
  assert.equal(controller.handleResize(), true)
  assert.deepEqual(resizedSessions, ["session-a"])

  sessionId = "session-b"
  controller.handleResize()
  assert.deepEqual(resizedSessions, ["session-a", "session-b"])
})

// The waiting room draws its rows to the terminal width of the moment it was built; a panel that is resized or
// maximized after the TUI started must redraw it, or its centered rows run past both edges.
test("terminal resize controller redraws the waiting room when no session is attached", () => {
  let rebuilds = 0
  const resizedSessions: string[] = []
  let attached = false
  const controller = createTerminalResizeController({
    isAttached: () => attached,
    sessionId: () => "session-a",
    resizeSession: (id) => { resizedSessions.push(id) },
    rebuildWaitingRoom: () => { rebuilds += 1 },
  })

  controller.handleResize()
  assert.equal(rebuilds, 1)
  assert.deepEqual(resizedSessions, [])

  attached = true
  controller.handleResize()
  assert.equal(rebuilds, 1, "an attached session resizes its terminal instead")
  assert.deepEqual(resizedSessions, ["session-a"])
})

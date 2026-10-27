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

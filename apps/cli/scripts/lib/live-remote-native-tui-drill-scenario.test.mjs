import assert from "node:assert/strict"
import test from "node:test"

import {
  attachSubscribedTerminalClient,
} from "./live-remote-native-tui-drill-scenario.mjs"

test("remote native drill subscribes its terminal attachment before using it", async () => {
  const calls = []
  const client = {
    async send(request) {
      calls.push({ operation: "send", request })
      return {
        SessionAttached: {
          attachment: { id: "attachment-observer" },
        },
      }
    },
    async subscribeToKernelEvents(sessionId, attachmentId) {
      calls.push({ operation: "subscribe", sessionId, attachmentId })
    },
  }

  const attachment = await attachSubscribedTerminalClient(
    client,
    "session-remote",
    "remote-native-drill",
  )

  assert.equal(attachment.id, "attachment-observer")
  assert.deepEqual(calls, [
    {
      operation: "send",
      request: {
        AttachToSession: {
          session_id: "session-remote",
          client_id: "remote-native-drill",
          capability_level: "FullTerminal",
        },
      },
    },
    {
      operation: "subscribe",
      sessionId: "session-remote",
      attachmentId: "attachment-observer",
    },
  ])
})

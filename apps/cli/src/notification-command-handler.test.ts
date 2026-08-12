import assert from "node:assert/strict"
import test from "node:test"

import { handleNotificationSlashCommand } from "./notification-command-handler.js"

test("TUI notification command renders installed services without session state", async () => {
  const notices: string[] = []
  await handleNotificationSlashCommand({
    sendWorkflowEventPublicationRequest: async () => ({ EventConnectionsPage: { page: {
      connections: [{
        generator_id: "dev.arroba.github",
        connection_id: "connection-1",
        status: "ready",
        created_at_ms: 1,
        updated_at_ms: 2,
      }],
      next_cursor: null,
    } } }),
    appendNotice: (message) => notices.push(message),
    flashFooter: (message) => assert.fail(message),
  }, {
    kind: "notifications",
    raw: "/notifications connections",
    args: ["connections"],
  })

  assert.match(notices[0] ?? "", /connection-1  ready/)
})

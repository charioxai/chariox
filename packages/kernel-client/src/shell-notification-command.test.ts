import assert from "node:assert/strict"
import test from "node:test"

import { executeNotificationCommand } from "./shell-notification-command.js"

test("notification center installs and reuses kernel-owned connections", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("InstallEventConnection" in request) {
        return { EventConnectionAuthorizationStarted: { authorization: {
          authorization_id: "authorization-1",
          generator_id: "dev.chariox.github",
          status: "user_action_required",
          authorization_url: "https://example.test/authorize",
          user_code: "ABCD-1234",
          created_at_ms: 1,
        } } }
      }
      return { EventConnectionsPage: { page: {
        connections: [{
          generator_id: "dev.chariox.github",
          connection_id: "connection-1",
          status: "ready",
          created_at_ms: 1,
          updated_at_ms: 2,
          last_validated_at_ms: 2,
        }],
        next_cursor: null,
      } } }
    },
  }

  const installed = await executeNotificationCommand(["connect", "dev.chariox.github"], client)
  const listed = await executeNotificationCommand(["connections", "dev.chariox.github"], client)

  assert.match(installed.message ?? "", /ABCD-1234/)
  assert.match(listed.message ?? "", /connection-1  ready/)
  assert.deepEqual(requests, [
    { InstallEventConnection: { generator_id: "dev.chariox.github", return_url: null } },
    { ListEventConnections: { generator_id: "dev.chariox.github", cursor: null, limit: 20 } },
  ])
})

test("notification removal previews dependencies before confirmed removal", async () => {
  const requests: Record<string, unknown>[] = []
  const dependency = {
    session_id: "session-1",
    publication_id: "publication-1",
    binding_id: "binding-1",
    status: "active" as const,
  }
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("ListEventConnectionDependencies" in request) {
        return { EventConnectionDependencies: { connection_id: "connection-1", dependencies: [dependency] } }
      }
      return { EventConnectionRemoved: {
        connection: {
          generator_id: "dev.chariox.github",
          connection_id: "connection-1",
          status: "revoked",
          created_at_ms: 1,
          updated_at_ms: 2,
        },
        deactivated_bindings: [dependency],
      } }
    },
  }

  const preview = await executeNotificationCommand(["connection", "remove", "connection-1"], client)
  assert.match(preview.message ?? "", /publication=publication-1/)
  assert.match(preview.message ?? "", /--confirm/)
  assert.equal(requests.length, 1)

  const removed = await executeNotificationCommand(
    ["connection", "remove", "connection-1", "--confirm"],
    client,
  )
  assert.match(removed.message ?? "", /removed connection-1/)
  assert.deepEqual(requests.slice(1), [
    { ListEventConnectionDependencies: { connection_id: "connection-1" } },
    { RemoveEventConnection: { connection_id: "connection-1", confirm: true } },
  ])
})

import assert from "node:assert/strict"
import test from "node:test"

import { executeNotificationCommand } from "./shell-notification-command.js"
import { executePromptSettingsCommand } from "./shell-prompt-settings-command.js"

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
          lifecycle_state: "connected",
          attached_trigger_count: 1,
          test_event_supported: true,
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
  assert.match(listed.message ?? "", /connection-1  connected/)
  assert.deepEqual(requests, [
    { InstallEventConnection: { generator_id: "dev.chariox.github", return_url: null } },
    { ListEventConnections: { generator_id: "dev.chariox.github", cursor: null, limit: 20 } },
  ])
})

test("prompt settings slash command resets one setting with optimistic versioning", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("ListPromptSettings" in request) {
        return { PromptSettingsListed: { settings: [{
          id: "workflow/turn",
          title: "Workflow turn contract",
          current_sha256: "sha-current",
          revision: 3,
          editable: true,
          protected: false,
          source: "user_override",
        }] } }
      }
      return { PromptSetting: { setting: {
        id: "workflow/turn",
        title: "Workflow turn contract",
        current_sha256: "sha-default",
        revision: 4,
        editable: true,
        protected: false,
        source: "bundled",
      } } }
    },
  }
  const result = await executePromptSettingsCommand(["reset", "workflow/turn", "--confirm"], client)
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /reset workflow\/turn/)
  assert.deepEqual(requests, [
    { ListPromptSettings: null },
    { ResetPromptSetting: {
      id: "workflow/turn",
      expected_revision: 3,
      expected_sha256: "sha-current",
    } },
  ])
})

test("prompt settings reset-all requires explicit confirmation and uses every catalog version", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("ListPromptSettings" in request) {
        return { PromptSettingsListed: { settings: [
          { id: "a", current_sha256: "sha-a", revision: 1, editable: true, protected: false, source: "user_override" },
          { id: "b", current_sha256: "sha-b", revision: 2, editable: false, protected: true, source: "bundled" },
        ] } }
      }
      return { PromptSettingsReset: { settings: [] } }
    },
  }
  const denied = await executePromptSettingsCommand(["reset-all"], client)
  assert.equal(denied.ok, false)
  assert.match(denied.message ?? "", /--confirm/)
  assert.equal(requests.length, 0)
  const result = await executePromptSettingsCommand(["reset-all", "--confirm"], client)
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { ListPromptSettings: null },
    { ResetAllPromptSettings: { expected: {
      a: { revision: 1, sha256: "sha-a" },
      b: { revision: 2, sha256: "sha-b" },
    } } },
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
  assert.match(preview.message ?? "", /trigger-owner=publication-1/)
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

test("notification connection test uses the shared kernel lifecycle request", async () => {
  const requests: Record<string, unknown>[] = []
  const result = await executeNotificationCommand(
    ["connection", "test", "connection-1", "pull_request.opened"],
    {
      send: async (request) => {
        requests.push(request)
        return { EventConnectionTested: { result: {
          occurrence_id: "test-occurrence-1",
          accepted: true,
        } } }
      },
    },
  )
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /test-occurrence-1/)
  assert.deepEqual(requests, [{ TestEventConnection: {
    connection_id: "connection-1",
    event_type: "pull_request.opened",
  } }])
})

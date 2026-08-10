import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import { handleRelayCloudCommand } from "./relay-cloud-command-handlers.js"
import type { RelayCloudProfile } from "./preferences.js"

test("relay cloud status reports missing cloud link", async () => {
  let notice = ""

  await handleRelayCloudCommand({
    appendNotice: (message) => { notice = message },
    flashFooter: () => {},
    formatError: (error) => String(error),
    sessionState: () => session(),
    getCloudRelayProfile: () => null,
  }, ["status"])

  assert.equal(notice, "cloud is not linked. Run /cloud first.")
})

test("relay cloud client-token pairs a client and emits the relay command", async () => {
  const unpaired = profile()
  const paired = profile({ clientId: "client-1" })
  const notices: string[] = []
  let savedClientId: string | undefined
  let issuedSessionId: string | null | undefined

  await handleRelayCloudCommand({
    appendNotice: (message) => { notices.push(message) },
    flashFooter: () => {},
    formatError: (error) => String(error),
    clientId: "cli-1",
    sessionState: () => session(),
    getCloudRelayProfile: () => unpaired,
    saveCloudRelayProfile: async (nextProfile) => { savedClientId = nextProfile?.clientId },
    pairCloudRelayClient: async (_profile, clientId) => profile({ clientId }),
    issueCloudClientRelayToken: async (_profile, _targetDaemonAlias, options) => {
      issuedSessionId = options?.sessionId
      return {
        relayUrl: "wss://relay.example",
        relayToken: "relay-token",
        tokenExpiresAtMs: 123,
        profile: paired,
      }
    },
  }, ["client-token", "builder-kernel"])

  assert.equal(savedClientId, "client-1")
  assert.equal(issuedSessionId, "session-1")
  assert.match(notices[0] ?? "", /command=arroba --relay-url wss:\/\/relay\.example --relay-token relay-token --target-daemon-alias builder-kernel/)
  assert.equal(notices.at(-1), "cloud client token minted for builder-kernel")
})

function profile(overrides: Partial<RelayCloudProfile> = {}): RelayCloudProfile {
  return {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "team",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: ["attachment-1"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    workflows: [],
    workflow_runs: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

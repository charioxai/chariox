import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import {
  buildCloudInviteUrl,
  handleCloudSessionCommand,
  parseCloudInviteReference,
} from "./cloud-session-command-handlers.js"
import type { RelayCloudProfile } from "./preferences.js"

test("cloud invite urls carry cloud and local invite tokens", () => {
  assert.equal(
    buildCloudInviteUrl("https://cloud.example", "cloud-token", "local-token"),
    "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token",
  )
})

test("cloud invite references parse urls and plain tokens", () => {
  assert.deepEqual(
    parseCloudInviteReference("https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token"),
    { cloudInviteToken: "cloud-token", localInviteToken: "local-token" },
  )
  assert.deepEqual(parseCloudInviteReference("plain-cloud-token"), { cloudInviteToken: "plain-cloud-token" })
})

test("cloud session invite create pairs cloud and local invite tokens", async () => {
  const currentSession = session({ alias: "review-session" })
  let createdMaxUses: number | null | undefined
  let cloudDisplayName: string | null | undefined
  let openedUrl: string | null = null
  let notice = ""
  let flashedMessage = ""

  const handled = await handleCloudSessionCommand({
    isAttached: () => true,
    sessionState: () => currentSession,
    applySessionState: () => {},
    attachBinding: async () => {},
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notice = message },
    openExternalUrl: async (url) => {
      openedUrl = url
      return false
    },
    createSessionInvite: async (_sessionId, _expiresInMs, maxUses) => {
      createdMaxUses = maxUses
      return {
        invite: { invite_token: "local-token", invite: { invite_id: "local-invite" } },
        session: currentSession,
      }
    },
    createCloudSessionInvite: async (_sessionId, options) => {
      cloudDisplayName = options.displayName
      return { invite: { invite_id: "cloud-invite", invite_token: "cloud-token" } }
    },
  }, profile(), "invite", "create", ["--max-uses", "2"])

  assert.equal(handled, true)
  assert.equal(createdMaxUses, 2)
  assert.equal(cloudDisplayName, "review-session")
  assert.equal(openedUrl, "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token")
  assert.match(notice, /cloud_invite_id=cloud-invite/)
  assert.equal(flashedMessage, "cloud invite created")
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

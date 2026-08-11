import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import {
  handleRelaySlashCommand,
} from "./cloud-command-handlers.js"
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
  let createdLevel: string | null | undefined
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
    createSessionInvite: async (_sessionId, _expiresInMs, maxUses, collaborationLevel) => {
      createdMaxUses = maxUses
      createdLevel = collaborationLevel
      return {
        invite: { invite_token: "local-token", invite: { invite_id: "local-invite" } },
        session: currentSession,
      }
    },
    createCloudSessionInvite: async (_sessionId, options) => {
      cloudDisplayName = options.displayName
      assert.equal(options.collaborationLevel, "transparent")
      return { invite: { invite_id: "cloud-invite", invite_token: "cloud-token" } }
    },
  }, profile(), "invite", "create", ["--max-uses", "2", "--level", "transparent"])

  assert.equal(handled, true)
  assert.equal(createdMaxUses, 2)
  assert.equal(createdLevel, "transparent")
  assert.equal(cloudDisplayName, "review-session")
  assert.equal(openedUrl, "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token")
  assert.match(notice, /cloud_invite_id=cloud-invite/)
  assert.equal(flashedMessage, "cloud invite created")
})

test("cloud invite accept refuses direct local attach for cloud collaborator identity", async () => {
  let accepted = false
  let joined = false
  let flashedMessage = ""

  const handled = await handleCloudSessionCommand({
    isAttached: () => false,
    sessionState: () => session(),
    applySessionState: () => {},
    attachBinding: async () => { throw new Error("attach should not be called") },
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    isRelayConnection: () => false,
    acceptCloudSessionInvite: async () => {
      accepted = true
      return { acceptance: { user_id: "user-2" } }
    },
    joinSessionInvite: async () => {
      joined = true
      return { member: { user_id: "user-2" }, session: session() }
    },
  }, profile({ userId: "user-2" }), "invite", "accept", [
    "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token",
  ])

  assert.equal(handled, true)
  assert.equal(accepted, false)
  assert.equal(joined, false)
  assert.equal(flashedMessage, "cloud invite accepted only through relay identity; reconnect with the relay invite link")
})

test("cloud invite accept joins when the TUI is relay connected", async () => {
  let attached = false
  let flashedMessage = ""

  const handled = await handleCloudSessionCommand({
    isAttached: () => false,
    sessionState: () => session(),
    applySessionState: () => {},
    attachBinding: async (joinedSession) => {
      assert.equal(joinedSession.id, "joined-session")
      attached = true
    },
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    isRelayConnection: () => true,
    acceptCloudSessionInvite: async (inviteToken) => {
      assert.equal(inviteToken, "cloud-token")
      return { acceptance: { user_id: "user-2" } }
    },
    joinSessionInvite: async (inviteToken, userId) => {
      assert.equal(inviteToken, "local-token")
      assert.equal(userId, "user-2")
      return { member: { user_id: "user-2" }, session: session({ id: "joined-session" }) }
    },
  }, profile({ userId: "user-2" }), "invite", "accept", [
    "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token",
  ])

  assert.equal(handled, true)
  assert.equal(attached, true)
  assert.equal(flashedMessage, "joined cloud session as user-2")
})

test("relay invite create uses local kernel invite without cloud metadata", async () => {
  const currentSession = session()
  let notice = ""
  let flashedMessage = ""
  let createdMaxUses: number | null | undefined

  await handleRelaySlashCommand({
    isAttached: () => true,
    sessionState: () => currentSession,
    applySessionState: () => {},
    attachBinding: async () => {},
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    appendNotice: (message) => { notice = message },
    createSessionInvite: async (_sessionId, _expiresInMs, maxUses) => {
      createdMaxUses = maxUses
      return {
        invite: { invite_token: "relay-token", invite: { invite_id: "relay-invite" } },
        session: currentSession,
      }
    },
  }, {
    kind: "relay",
    raw: "/relay invite create 2",
    args: ["invite", "create", "2"],
  })

  assert.equal(createdMaxUses, 2)
  assert.match(notice, /invite_token=relay-token/)
  assert.doesNotMatch(notice, /cloud_invite/)
  assert.equal(flashedMessage, "relay invite created")
})

test("relay invite accept joins as same-relay daemon identity", async () => {
  let attached = false
  let flashedMessage = ""

  await handleRelaySlashCommand({
    isAttached: () => false,
    sessionState: () => session(),
    applySessionState: () => {},
    attachBinding: async (joinedSession) => {
      assert.equal(joinedSession.id, "joined-session")
      attached = true
    },
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    appendNotice: () => {},
    getRelayStatus: async () => ({
      configured: true,
      connected: true,
      relay_url: "ws://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-collaborator",
      machine_id: "machine-1",
      machine_alias: "collab",
    }),
    joinSessionInvite: async (inviteToken, userId) => {
      assert.equal(inviteToken, "relay-token")
      assert.equal(userId, "daemon-collaborator")
      return { member: { user_id: userId }, session: session({ id: "joined-session" }) }
    },
  }, {
    kind: "relay",
    raw: "/relay invite accept relay-token",
    args: ["invite", "accept", "relay-token"],
  })

  assert.equal(attached, true)
  assert.equal(flashedMessage, "joined relay session as daemon-collaborator")
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

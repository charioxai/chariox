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
  assert.match(notices[0] ?? "", /command=chariox --relay-url wss:\/\/relay\.example --relay-token relay-token --target-daemon-alias builder-kernel/)
  assert.equal(notices.at(-1), "cloud client token minted for builder-kernel")
})

test("explicit Cloud revocation preserves the link when acknowledgement fails", async () => {
  for (const flag of ["--revoke-machine", "--revoke-client"]) {
    const linked = profile({ machineId: "machine-1", clientId: "client-1" })
    let current: RelayCloudProfile | null = linked
    const notices: string[] = []
    await assert.rejects(handleRelayCloudCommand({
      appendNotice: (message) => { notices.push(message) },
      flashFooter: () => {},
      formatError: (error) => String(error),
      sessionState: () => session(),
      getCloudRelayProfile: () => current,
      saveCloudRelayProfile: async (next) => { current = next },
      logoutCloudRelay: async () => { throw new Error("Cloud acknowledgement unavailable") },
    }, ["logout", flag]), /Cloud acknowledgement unavailable/)
    assert.equal(current, linked)
    assert.equal(notices.includes("cloud link cleared"), false)
  }
})

test("explicit Cloud revocation clears the link only after acknowledgement", async () => {
  const linked = profile({ machineId: "machine-1", clientId: "client-1" })
  let current: RelayCloudProfile | null = linked
  const notices: string[] = []
  let acknowledge!: () => void
  const acknowledgement = new Promise<void>((resolve) => { acknowledge = resolve })
  const logout = handleRelayCloudCommand({
    appendNotice: (message) => { notices.push(message) },
    flashFooter: () => {},
    formatError: (error) => String(error),
    sessionState: () => session(),
    getCloudRelayProfile: () => current,
    saveCloudRelayProfile: async (next) => { current = next },
    logoutCloudRelay: async (_profile, options) => {
      assert.deepEqual(options, { revokeClient: true, revokeMachine: true })
      await acknowledgement
    },
  }, ["logout", "--revoke-machine", "--revoke-client"])
  await Promise.resolve()
  assert.equal(current, linked)
  assert.equal(notices.includes("cloud link cleared"), false)
  acknowledge()
  await logout
  assert.equal(current, null)
  assert.equal(notices.at(-1), "cloud link cleared")
})

test("explicit Cloud revocation rejects an unavailable link or logout capability", async () => {
  for (const availableProfile of [false, true]) {
    const linked = availableProfile ? profile({ machineId: "machine-1" }) : null
    let current: RelayCloudProfile | null = linked
    const notices: string[] = []
    await assert.rejects(handleRelayCloudCommand({
      appendNotice: (message) => { notices.push(message) },
      flashFooter: () => {},
      formatError: (error) => String(error),
      sessionState: () => session(),
      getCloudRelayProfile: () => current,
      saveCloudRelayProfile: async (next) => { current = next },
      ...(availableProfile ? {} : { logoutCloudRelay: async () => { throw new Error("must not request Cloud without a profile") } }),
    }, ["disable", "--revoke-machine"]), /requires a linked profile and logout support/)
    assert.equal(current, linked)
    assert.equal(notices.includes("cloud link cleared"), false)
  }
})

test("plain Cloud logout clears the local link while offline", async () => {
  let current: RelayCloudProfile | null = profile()
  const notices: string[] = []
  await handleRelayCloudCommand({
    appendNotice: (message) => { notices.push(message) },
    flashFooter: () => {},
    formatError: (error) => String(error),
    sessionState: () => session(),
    getCloudRelayProfile: () => current,
    saveCloudRelayProfile: async (next) => { current = next },
    logoutCloudRelay: async () => { throw new Error("offline") },
  }, ["logout"])
  assert.equal(current, null)
  assert.match(notices[0] ?? "", /offline/)
  assert.equal(notices.at(-1), "cloud link cleared")
})

test("plain Cloud logout is idempotent without a link", async () => {
  const notices: string[] = []
  await handleRelayCloudCommand({
    appendNotice: (message) => { notices.push(message) },
    flashFooter: () => {},
    formatError: (error) => String(error),
    sessionState: () => session(),
    getCloudRelayProfile: () => null,
    saveCloudRelayProfile: async (next) => { assert.equal(next, null) },
    logoutCloudRelay: async () => { throw new Error("must not request Cloud without a profile") },
  }, ["logout"])
  assert.deepEqual(notices, ["cloud link cleared"])
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

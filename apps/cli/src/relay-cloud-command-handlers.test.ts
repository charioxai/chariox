import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import { handleRelayCloudCommand } from "./relay-cloud-command-handlers.js"
import { createInitialCloudClientTokenIssuer } from "./cloud-client-token-issuer.js"
import { parseArgs } from "./cli-options.js"
import type { LocalIpcClient } from "./ipc.js"
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

test("relay cloud client-token lets kernel issuance pair the client and preserves alias commands", async () => {
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
    pairCloudRelayClient: async () => { throw new Error("client-token must not pre-pair an account client") },
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

test("machine-only client-token uses shared kernel issuance and launches its canonical scoped target", async () => {
  const machineOnly = profile({ machineId: "machine-1", machineCredential: "synthetic-machine-credential" })
  const fixture = JSON.parse(await readFile(new URL("../../../fixtures/machine-client-cli-relay-target.json", import.meta.url), "utf8")) as {
    requestedAlias: string; claims: { public_key_thumbprint: string; allowed_targets: string[] };
    syntheticToken: string; tokenExpiresAt: string; generatedCommand: string;
    parsedTarget: { daemonId: string; daemonAlias: null };
  }
  const canonicalTarget = fixture.parsedTarget.daemonId
  const requestedAlias = fixture.requestedAlias
  const claims = fixture.claims
  const token = fixture.syntheticToken
  assert.equal(token, `eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.synthetic-signature`)
  const requests: unknown[] = []
  const notices: string[] = []
  const saved: RelayCloudProfile[] = []
  const client = { send: async (request: unknown) => {
    requests.push(request)
    assert.deepEqual(request, { IssueCloudRelayClientToken: {
      target_daemon_alias: requestedAlias, client_id: "cli-1", session_id: "session-1",
      public_key_thumbprint: "fixture-thumbprint",
    } })
    return { CloudRelayClientTokenIssued: {
      profile: {
        api_url: machineOnly.apiUrl, email: machineOnly.email,
        account_id: machineOnly.accountId, user_id: machineOnly.userId,
        account_slug: machineOnly.accountSlug, realm_id: machineOnly.realmId,
        relay_url: machineOnly.relayUrl, issuer_id: machineOnly.issuerId,
        machine_id: machineOnly.machineId, machine_credential: machineOnly.machineCredential,
        client_id: "machine-scoped-client-1",
      },
      token: { relay_url: machineOnly.relayUrl, relay_token: token, token_expires_at: fixture.tokenExpiresAt },
    } }
  } } as unknown as LocalIpcClient
  await handleRelayCloudCommand({
    appendNotice: (message) => { notices.push(message) }, flashFooter: () => {},
    formatError: (error) => String(error), clientId: "cli-1", sessionState: () => session(),
    getCloudRelayProfile: () => machineOnly,
    saveCloudRelayProfile: async (next) => { if (next) saved.push(next) },
    pairCloudRelayClient: async () => { throw new Error("machine-only profile cannot account-pair a client") },
    issueCloudClientRelayToken: createInitialCloudClientTokenIssuer(client, "cli-1",
      () => ({ publicKeyThumbprint: "fixture-thumbprint" })),
  }, ["client-token", requestedAlias])
  assert.equal(requests.length, 1)
  assert.equal(saved.length, 1)
  assert.equal(saved[0]?.clientId, "machine-scoped-client-1")
  assert.equal(machineOnly.clientId, undefined)
  const command = notices[0]?.split("\n").find((line) => line.startsWith("command="))
  assert.ok(command)
  assert.equal(command, `command=${fixture.generatedCommand}`)
  const options = parseArgs(command.slice("command=chariox ".length).split(" "))
  assert.equal(options.targetDaemonId, canonicalTarget)
  assert.equal(options.targetDaemonAlias ?? null, fixture.parsedTarget.daemonAlias)
  assert.equal(options.relayToken, token)
  assert.deepEqual(claims.allowed_targets, [options.targetDaemonId])
  assert.equal(notices.at(-1), `cloud client token minted for ${requestedAlias}`)
})

test("restarted TUI client-token command issues for its saved paired login identity", async () => {
  const linked = profile({ clientId: "paired-login-client" })
  const restarted = parseArgs([])
  assert.equal(restarted.clientId, `chariox-cli-${process.pid}`)
  assert.notEqual(restarted.clientId, linked.clientId)
  const requests: unknown[] = []
  const claims = { sub: linked.clientId, client_id: linked.clientId, subject_kind: "CLIENT",
    public_key_thumbprint: "restart-key", allowed_targets: ["kernel-1"] }
  const token = `eyJhbGciOiJIUzI1NiJ9.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.synthetic`
  const client = { socketPath: "ws://127.0.0.1:49911/kernel", send: async (request: { RelayStatus?: null; IssueCloudRelayClientToken?: { client_id: string } }) => {
    requests.push(request)
    if ("RelayStatus" in request) return { RelayStatus: {status: {daemon_id: "issuing-kernel"}} }
    // Session-authenticated Cloud issuance refuses a different, unpaired subject.
    assert.equal(request.IssueCloudRelayClientToken?.client_id, linked.clientId, "identity_revoked")
    return { CloudRelayClientTokenIssued: {
      profile: { api_url: linked.apiUrl, email: linked.email, account_id: linked.accountId,
        user_id: linked.userId, account_slug: linked.accountSlug, realm_id: linked.realmId,
        relay_url: linked.relayUrl, issuer_id: linked.issuerId, client_id: linked.clientId },
      token: { relay_url: linked.relayUrl, relay_token: token, token_expires_at: "2030-01-01T00:00:00Z" },
    } }
  } } as unknown as LocalIpcClient
  const notices: string[] = []
  await handleRelayCloudCommand({
    appendNotice: (notice) => { notices.push(notice) }, flashFooter: () => {},
    formatError: String, clientId: restarted.clientId, sessionState: () => session(),
    getCloudRelayProfile: () => linked, saveCloudRelayProfile: async () => {},
    pairCloudRelayClient: async () => { throw new Error("must not re-pair the saved login") },
    issueCloudClientRelayToken: createInitialCloudClientTokenIssuer(client, restarted.clientId!,
      () => ({ publicKeyThumbprint: "restart-key" })),
  }, ["client-token", "builder-kernel"])
  assert.deepEqual(requests, [{ IssueCloudRelayClientToken: {
    target_daemon_alias: "builder-kernel", client_id: linked.clientId,
    session_id: "session-1", public_key_thumbprint: "restart-key",
  } }, {RelayStatus: null}])
  assert.equal(notices.at(-1), "cloud client token minted for builder-kernel")
  assert.match(notices[0]!, /--target-daemon-id kernel-1/)
  assert.match(notices[0]!, /--relay-token-issuer/)
  const commandOptions = parseArgs(["--relay-url", "wss://relay.example", "--relay-token", "synthetic", "--target-daemon-id", "kernel-1", "--relay-token-issuer", "ws://127.0.0.1:49911/kernel", "issuing-kernel"])
  assert.deepEqual(commandOptions.relayTokenIssuer, {endpoint: "ws://127.0.0.1:49911/kernel", daemonId: "issuing-kernel"})
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

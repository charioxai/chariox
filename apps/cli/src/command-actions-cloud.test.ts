import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, QueuedWorkflowLaunch, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession, runGit } from "./command-actions-test-support.js"

test("relay cloud login stores the bootstrap profile", async () => {
  const notices: string[] = []
  let savedProfile: Record<string, unknown> | null = null
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendNotice: (message: string) => { notices.push(message) },
    bootstrapCloudRelay: async (apiUrl: string, email: string, accountSlug?: string) => ({
      apiUrl,
      email,
      accountId: "account-1",
      userId: "user-1",
      accountSlug: accountSlug ?? "user",
      realmId: "realm-1",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
    }),
    saveCloudRelayProfile: async (profile: Record<string, unknown> | null) => {
      savedProfile = profile
    },
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud login", args: ["cloud", "login", "https://cloud.example", "user@example.com", "user"] })

  assert.equal((savedProfile as { relayUrl?: string } | null)?.relayUrl, "wss://relay.example")
  assert.equal(notices.at(-1), "cloud profile saved: user")
})

test("relay cloud login without args uses device flow", async () => {
  const notices: string[] = []
  const savedProfiles: Array<Record<string, unknown> | null> = []
  const configured: Array<{ relayUrl: string | null; relayToken: string | null }> = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    clientId: "client-1",
    appendNotice: (message: string) => { notices.push(message) },
    bootstrapCloudRelay: async () => {
      throw new Error("bootstrap path should not be used")
    },
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async (apiUrl: string, input: { clientId?: string; machineId?: string; machineAlias?: string }) => {
      assert.equal(apiUrl, "https://arroba-cloud-staging.osc-fr1.scalingo.io")
      assert.equal(input.clientId, "client-1")
      assert.equal(input.machineId, "machine-1")
      assert.equal(input.machineAlias, "laptop")
      return {
        apiUrl,
        deviceCode: "dev-code",
        userCode: "ABCD-EFGH",
        verificationUrl: "https://arroba-cloud-staging.osc-fr1.scalingo.io/activate?user_code=ABCD-EFGH",
        expiresAtMs: Date.now() + 60_000,
        intervalSeconds: 1,
      }
    },
    openExternalUrl: async () => false,
    pollCloudDeviceLogin: async (_apiUrl: string, deviceCode: string) => {
      assert.equal(deviceCode, "dev-code")
      return {
        status: "approved",
        profile: {
          apiUrl: "https://arroba-cloud-staging.osc-fr1.scalingo.io",
          email: "user@example.com",
          accountId: "account-1",
          userId: "user-1",
          accountSlug: "user",
          realmId: "realm-1",
          relayUrl: "wss://relay.example",
          issuerId: "issuer-1",
          clientId: "client-1",
          machineId: "machine-1",
          cloudSessionToken: "session-token",
        },
      }
    },
    pairCloudRelayMachine: async (profile: Record<string, unknown>, machineId: string, alias?: string) => ({
      ...profile,
      machineId,
      machineAlias: alias ?? "laptop",
    }),
    issueCloudMachineRelayToken: async (_profile: Record<string, unknown>, daemonId: string, machineId: string) => {
      assert.equal(daemonId, "daemon-1")
      assert.equal(machineId, "machine-1")
      return {
        relayUrl: "wss://relay.example",
        relayToken: "relay-token",
        tokenExpiresAtMs: 1234,
      }
    },
    configureRelay: async (relayUrl: string | null, relayToken: string | null) => {
      configured.push({ relayUrl, relayToken })
      return {
        configured: true,
        connected: true,
        relay_url: relayUrl,
        relay_token_configured: relayToken != null,
        daemon_id: "daemon-1",
        machine_id: "machine-1",
        machine_alias: "laptop",
      }
    },
    saveCloudRelayProfile: async (profile: Record<string, unknown> | null) => {
      savedProfiles.push(profile)
    },
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud login", args: ["cloud", "login"] })

  assert.equal((savedProfiles.at(-1) as { cloudSessionToken?: string; tokenExpiresAtMs?: number } | null)?.cloudSessionToken, "session-token")
  assert.equal((savedProfiles.at(-1) as { tokenExpiresAtMs?: number } | null)?.tokenExpiresAtMs, 1234)
  assert.deepEqual(configured, [{ relayUrl: "wss://relay.example", relayToken: "relay-token" }])
  assert.ok(notices.some((message) => /code=ABCD-EFGH/.test(message)))
  assert.equal(notices.at(-1), "cloud linked: user")
})

test("/cloud opens hosted browser terminal when a cloud profile already exists", async () => {
  const flashed: string[] = []
  const notices: string[] = []
  const openedUrls: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => { flashed.push(message) },
    appendCloudNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => ({
      apiUrl: "https://cloud.example",
      email: "user@example.com",
      accountId: "account-1",
      userId: "user-1",
      accountSlug: "user",
      realmId: "realm-1",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
    }),
    getRelayStatus: async () => ({
      configured: true,
      connected: true,
      relay_url: "wss://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async () => {
      throw new Error("device login should not start for an existing cloud profile")
    },
    openExternalUrl: async (url: string) => {
      openedUrls.push(url)
      return true
    },
  }))

  await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] })

  assert.deepEqual(openedUrls, ["https://cloud.example/terminal?view=waiting"])
  assert.match(notices[0] ?? "", /Opening Arroba Cloud/)
  assert.doesNotMatch(notices.join("\n"), /code=|Link this machine/)
  assert.equal(flashed.at(-1), "opened Arroba Cloud")
})

test("/cloud links instead of opening when the local kernel is not linked", async () => {
  const notices: string[] = []
  let startedDeviceLogin = false
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendCloudNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => ({
      apiUrl: "https://cloud.example",
      email: "user@example.com",
      accountId: "account-1",
      userId: "user-1",
      accountSlug: "user",
      realmId: "realm-1",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
    }),
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    issueCloudKernelRelayToken: async () => {
      throw new Error("cloud relay profile missing; run /relay cloud login first")
    },
    configureRelay: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async () => {
      startedDeviceLogin = true
      return {
        apiUrl: "https://cloud.example",
        deviceCode: "device-code",
        userCode: "ABCD-EFGH",
        verificationUrl: "https://cloud.example/activate?user_code=ABCD-EFGH",
        expiresAtMs: Date.now() + 60_000,
        intervalSeconds: 1,
      }
    },
    openExternalUrl: async () => false,
    pollCloudDeviceLogin: async () => ({ status: "expired_token" }),
    saveCloudRelayProfile: async () => {},
  }))

  await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] })

  assert.equal(startedDeviceLogin, true)
  assert.match(notices[0] ?? "", /Link this machine to Arroba Cloud/)
  assert.doesNotMatch(notices[0] ?? "", /Opening Arroba Cloud/)
})

test("/cloud links instead of opening when the cloud machine identity was revoked", async () => {
  const notices: string[] = []
  const openedUrls: string[] = []
  let startedDeviceLogin = false
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendCloudNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => ({
      apiUrl: "https://cloud.example",
      email: "user@example.com",
      accountId: "account-1",
      userId: "user-1",
      accountSlug: "user",
      realmId: "realm-1",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
      machineId: "machine-1",
    }),
    getRelayStatus: async () => ({
      configured: true,
      connected: false,
      relay_url: "wss://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    issueCloudMachineRelayToken: async () => {
      throw new Error('cloud relay request failed with 403: {"error":{"code":"identity_revoked","message":"Subject has been revoked"}}')
    },
    configureRelay: async () => ({
      configured: true,
      connected: false,
      relay_url: "wss://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async () => {
      startedDeviceLogin = true
      return {
        apiUrl: "https://cloud.example",
        deviceCode: "device-code",
        userCode: "ABCD-EFGH",
        verificationUrl: "https://cloud.example/activate?user_code=ABCD-EFGH",
        expiresAtMs: Date.now() + 60_000,
        intervalSeconds: 1,
      }
    },
    openExternalUrl: async (url: string) => {
      openedUrls.push(url)
      return true
    },
    pollCloudDeviceLogin: async () => ({ status: "expired_token" }),
    saveCloudRelayProfile: async () => {},
  }))

  await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud", args: [] })

  assert.equal(startedDeviceLogin, true)
  assert.deepEqual(openedUrls, ["https://cloud.example/activate?user_code=ABCD-EFGH"])
  assert.match(notices[0] ?? "", /Cloud link needs refresh/)
  assert.match(notices[1] ?? "", /Link this machine to Arroba Cloud/)
  assert.doesNotMatch(notices.join("\n"), /Opening Arroba Cloud/)
})

test("/cloud link triggers hosted device login flow", async () => {
  const flashed: string[] = []
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => { flashed.push(message) },
    appendNotice: (message: string) => { notices.push(message) },
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async () => ({
      apiUrl: "https://cloud.example",
      deviceCode: "device-code",
      userCode: "ABCD-EFGH",
      verificationUrl: "https://cloud.example/activate?user_code=ABCD-EFGH",
      expiresAtMs: Date.now() + 60_000,
      intervalSeconds: 1,
    }),
    openExternalUrl: async () => true,
    pollCloudDeviceLogin: async () => ({
      status: "approved",
      profile: {
        apiUrl: "https://cloud.example",
        email: "user@example.com",
        accountId: "account-1",
        userId: "user-1",
        accountSlug: "user",
        realmId: "realm-1",
        relayUrl: "wss://relay.example",
        issuerId: "issuer-1",
      },
    }),
    pairCloudRelayMachine: async (profile: Record<string, unknown>) => profile,
    issueCloudKernelRelayToken: async () => ({
      relayUrl: "wss://relay.example",
      relayToken: "relay-token",
      tokenExpiresAtMs: 1234,
    }),
    configureRelay: async () => ({
      configured: true,
      connected: true,
      relay_url: "wss://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    saveCloudRelayProfile: async () => {},
  }))

  await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud link", args: ["link"] })

  assert.deepEqual(flashed, [])
  assert.match(notices[0] ?? "", /Link this machine to Arroba Cloud/)
  assert.equal(notices.at(-1), "cloud linked: user")
})

test("relay cloud login without args prefers configured hosted api url", async () => {
  const handlers = createCommandActionHandlers(makeCommandDeps({
    clientId: "client-1",
    cloudRelayApiUrl: "https://arroba-cloud-staging.osc-fr1.scalingo.io/",
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    startCloudDeviceLogin: async (apiUrl: string) => {
      assert.equal(apiUrl, "https://arroba-cloud-staging.osc-fr1.scalingo.io/")
      return {
        apiUrl,
        deviceCode: "dev-code",
        userCode: "ABCD-EFGH",
        verificationUrl: "https://arroba-cloud-staging.osc-fr1.scalingo.io/activate?user_code=ABCD-EFGH",
        expiresAtMs: Date.now() + 60_000,
        intervalSeconds: 1,
      }
    },
    openExternalUrl: async () => true,
    pollCloudDeviceLogin: async () => ({ status: "expired_token" }),
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud login", args: ["cloud", "login"] })
})

test("relay cloud connect mints a daemon token and configures relay", async () => {
  const notices: string[] = []
  const configured: Array<{ relayUrl: string | null; relayToken: string | null }> = []
  let savedProfile: Record<string, unknown> | null = null
  const profile = {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "user",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
  }
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => profile,
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: null,
    }),
    issueCloudKernelRelayToken: async () => ({
      relayUrl: "wss://relay.example",
      relayToken: "runtime-token",
      tokenExpiresAtMs: 1234,
    }),
    configureRelay: async (relayUrl: string | null, relayToken: string | null) => {
      configured.push({ relayUrl, relayToken })
      return {
        configured: true,
        connected: true,
        relay_url: relayUrl,
        relay_token_configured: Boolean(relayToken),
        daemon_id: "daemon-1",
        machine_id: "machine-1",
        machine_alias: null,
      }
    },
    saveCloudRelayProfile: async (next: Record<string, unknown> | null) => {
      savedProfile = next
    },
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud connect", args: ["cloud", "connect"] })

  assert.deepEqual(configured, [{ relayUrl: "wss://relay.example", relayToken: "runtime-token" }])
  assert.equal((savedProfile as { tokenExpiresAtMs?: number } | null)?.tokenExpiresAtMs, 1234)
  assert.equal(notices.at(-1), "cloud kernel connected: wss://relay.example")
})

test("relay cloud connect reports pending relay registration clearly", async () => {
  const notices: string[] = []
  const profile = {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "user",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
  }
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => profile,
    cloudRelayConnectTimeoutMs: 1,
    cloudRelayConnectPollMs: 1,
    getRelayStatus: async () => ({
      configured: true,
      connected: false,
      relay_url: "wss://relay.example",
      relay_token_configured: true,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: null,
    }),
    issueCloudKernelRelayToken: async () => ({
      relayUrl: "wss://relay.example",
      relayToken: "runtime-token",
      tokenExpiresAtMs: 1234,
    }),
    configureRelay: async (relayUrl: string | null, relayToken: string | null) => ({
      configured: true,
      connected: false,
      relay_url: relayUrl,
      relay_token_configured: Boolean(relayToken),
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: null,
    }),
    saveCloudRelayProfile: async () => {},
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud connect", args: ["cloud", "connect"] })

  assert.match(notices.at(-1) ?? "", /kernel is not online in Cloud yet/)
  assert.match(notices.at(-1) ?? "", /relay=not connected/)
  assert.match(notices.at(-1) ?? "", /machine=machine-1/)
})

test("relay cloud pair-machine stores the local machine identity", async () => {
  const notices: string[] = []
  let savedProfile: Record<string, unknown> | null = null
  const profile = {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "user",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
  }
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendNotice: (message: string) => { notices.push(message) },
    getCloudRelayProfile: () => profile,
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    }),
    pairCloudRelayMachine: async (_profile: Record<string, unknown>, machineId: string, alias?: string) => ({
      ...profile,
      machineId,
      ...(alias ? { machineAlias: alias } : {}),
    }),
    saveCloudRelayProfile: async (next: Record<string, unknown> | null) => {
      savedProfile = next
    },
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud pair-machine", args: ["cloud", "pair-machine"] })

  assert.equal((savedProfile as { machineId?: string } | null)?.machineId, "machine-1")
  assert.equal((savedProfile as { machineAlias?: string } | null)?.machineAlias, "laptop")
  assert.equal(notices.at(-1), "cloud machine linked: machine-1")
})

test("relay cloud connect prefers paired machine tokens", async () => {
  const configured: Array<{ relayUrl: string | null; relayToken: string | null }> = []
  const profile = {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "user",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
    machineId: "machine-1",
  }
  const handlers = createCommandActionHandlers(makeCommandDeps({
    getCloudRelayProfile: () => profile,
    getRelayStatus: async () => ({
      configured: false,
      connected: false,
      relay_url: null,
      relay_token_configured: false,
      daemon_id: "daemon-1",
      machine_id: "machine-1",
      machine_alias: null,
    }),
    issueCloudKernelRelayToken: async () => {
      throw new Error("kernel token path should not be used")
    },
    issueCloudMachineRelayToken: async (_profile: Record<string, unknown>, daemonId: string, machineId: string) => ({
      relayUrl: "wss://relay.example",
      relayToken: `machine-token:${machineId}:${daemonId}`,
      tokenExpiresAtMs: 5678,
    }),
    configureRelay: async (relayUrl: string | null, relayToken: string | null) => {
      configured.push({ relayUrl, relayToken })
      return {
        configured: true,
        connected: true,
        relay_url: relayUrl,
        relay_token_configured: Boolean(relayToken),
        daemon_id: "daemon-1",
        machine_id: "machine-1",
        machine_alias: null,
      }
    },
    saveCloudRelayProfile: async () => {},
  }))

  await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud connect", args: ["cloud", "connect"] })

  assert.deepEqual(configured, [{ relayUrl: "wss://relay.example", relayToken: "machine-token:machine-1:daemon-1" }])
})

test("cloud invite create pairs cloud and local session invite tokens", async () => {
  const notices: string[] = []
  const flashed: string[] = []
  const profile = {
    apiUrl: "https://cloud.example",
    email: "owner@example.com",
    accountId: "account-1",
    userId: "owner-1",
    accountSlug: "owner",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
  }
  const handlers = createCommandActionHandlers(makeCommandDeps({
    getCloudRelayProfile: () => profile,
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: (message: string) => { flashed.push(message) },
    openExternalUrl: async (url: string) => {
      assert.match(url, /cloud_invite=cloud-token/)
      assert.match(url, /local_invite=local-token/)
      return false
    },
    createSessionInvite: async (sessionId: string, _expiresInMs: number | null, maxUses: number | null) => {
      assert.equal(sessionId, "session-1")
      assert.equal(maxUses, 2)
      return {
        invite: { invite_token: "local-token", invite: { invite_id: "local-invite-1" } },
        session: makeSession(),
      }
    },
    createCloudSessionInvite: async (sessionId: string, options: { maxUses?: number | null }) => {
      assert.equal(sessionId, "session-1")
      assert.equal(options.maxUses, 2)
      return { invite: { invite_id: "cloud-invite-1", invite_token: "cloud-token" } }
    },
  }))

  await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud invite create 2", args: ["invite", "create", "2"] })

  assert.match(notices.at(-1) ?? "", /local_invite=local-token/)
  assert.equal(flashed.at(-1), "cloud invite created")
})

test("cloud invite accept accepts cloud token and joins local session when URL carries both tokens", async () => {
  const flashed: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    getCloudRelayProfile: () => ({
      apiUrl: "https://cloud.example",
      email: "peer@example.com",
      accountId: "account-2",
      userId: "peer-1",
      accountSlug: "peer",
      realmId: "realm-2",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
    }),
    flashFooter: (message: string) => { flashed.push(message) },
    acceptCloudSessionInvite: async (inviteToken: string) => {
      assert.equal(inviteToken, "cloud-token")
      return { acceptance: { user_id: "peer-1" } }
    },
    joinSessionInvite: async (inviteToken: string, userId: string) => {
      assert.equal(inviteToken, "local-token")
      assert.equal(userId, "peer-1")
      return {
        member: { user_id: "peer-1" },
        session: makeSession({ id: "joined-session" }),
      }
    },
  }))

  await handlers.handleCloudCommand({
    kind: "cloud",
    raw: "/cloud invite accept https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token",
    args: ["invite", "accept", "https://cloud.example/sessions/invites?cloud_invite=cloud-token&local_invite=local-token"],
  })

  assert.equal(flashed.at(-1), "joined cloud session as peer-1")
})

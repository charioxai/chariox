import { strict as assert } from "node:assert"
import { mkdtempSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import type { BootstrapState, CliOptions } from "./cli-types.js"
import type { CharioxPreferences } from "./preferences.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"
import {
  bootstrapCliRuntime,
  buildDetachedBootstrap,
  type CliRuntimeBootstrapDeps,
} from "./cli-runtime-bootstrap.js"

test("bootstrapCliRuntime falls back to detached waiting room when the default kernel is unreachable", async () => {
  const calls: string[] = []
  const options = cliOptions()
  const deps = createDeps({
    parseArgs: () => options,
    isNoArgDefaultKernelLaunch: () => true,
    isKernelEndpointReachable: async () => false,
    bootstrapAttachedSession: async () => {
      calls.push("bootstrapAttachedSession")
      throw new Error("should not attach")
    },
  })

  const result = await bootstrapCliRuntime({ argv: [], cwd: "/repo" }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(result.bootstrap.binding, null)
  assert.equal(result.bootstrap.options.detached, true)
  assert.equal(result.bootstrap.themeRegistry, DEFAULT_THEME_REGISTRY)
  assert.deepEqual(calls, [])
})

test("bootstrapCliRuntime attaches and resizes attached sessions", async () => {
  const calls: string[] = []
  const options = cliOptions({ kernelUrl: "ws://kernel.example/session" })
  const client = fakeClient()
  const deps = createDeps({
    parseArgs: () => options,
    createClient: () => client,
    bootstrapAttachedSession: async (attachedClient, attachedOptions, workspace, worktree, preferences) => {
      calls.push(`attach:${workspace}:${worktree}:${attachedOptions.clientId}:${attachedClient === client}`)
      return attachedBootstrap(attachedClient, attachedOptions, preferences)
    },
    maybeResize: async (resizeClient, sessionId) => {
      calls.push(`resize:${sessionId}:${resizeClient === client}`)
    },
  })

  const result = await bootstrapCliRuntime({ argv: ["--kernel-url", "ws://kernel.example/session"], cwd: "/repo" }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(result.kernelEndpoint, "ws://kernel.example/session")
  assert.equal(result.bootstrap.binding?.session.id, "session-1")
  assert.equal(result.bootstrap.themeRegistry, DEFAULT_THEME_REGISTRY)
  assert.deepEqual(calls, [
    "attach:/repo:/repo:cli-1:true",
    "resize:session-1:true",
  ])
})

test("bootstrapCliRuntime deletes a requested session without attaching", async () => {
  const calls: string[] = []
  const client = {
    close: async () => {
      calls.push("close")
    },
  } as LocalIpcClient
  const options = cliOptions({
    deleteSessionRef: "old-session",
    workspace: "/workspace",
  })
  const deps = createDeps({
    parseArgs: () => options,
    createClient: () => client,
    deleteSessionByRef: async (_client, sessionRef, workspace) => {
      calls.push(`delete:${sessionRef}:${workspace}`)
    },
    bootstrapAttachedSession: async () => {
      calls.push("bootstrapAttachedSession")
      throw new Error("should not attach")
    },
    maybeResize: async () => {
      calls.push("resize")
    },
  })

  const result = await bootstrapCliRuntime({ argv: ["--delete-session", "old-session"], cwd: "/repo" }, deps)

  assert.equal(result.kind, "deleted_session")
  assert.equal(result.workspace, "/workspace")
  assert.deepEqual(calls, ["delete:old-session:/workspace", "close"])
})

test("terminal pairing bootstrap replaces the legacy token with one bound to the stored CLI key", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-bootstrap-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  const tokenPayload = Buffer.from(JSON.stringify({
    public_key_thumbprint: identity.publicKeyThumbprint,
  })).toString("base64url")
  const boundRelayToken = `eyJhbGciOiJub25lIn0.${tokenPayload}.signature`
  const capturedRequests: unknown[] = []
  const createOptions: Array<{ relayAuthToken?: string; relayIdentity?: unknown }> = []
  let closedInitialClient = false
  const initialClient = {
    async send<TResponse>(request: unknown) {
      capturedRequests.push(request)
      return {
        TerminalPairingLinkJoined: {
          terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
          pairing: {
            intent: "client",
            subject_id: "terminal-1",
            relay_url: "wss://relay.example",
            target_daemon_id: "home-1",
            public_key_thumbprint: identity.publicKeyThumbprint,
            paired_at_ms: 1,
          },
          relay_token: boundRelayToken,
        },
      } as TResponse
    },
    close: async () => { closedInitialClient = true },
  } as LocalIpcClient
  const attachedClient = fakeClient()
  const options = cliOptions({
    clientId: "terminal-1",
    relayUrl: "wss://relay.example",
    relayToken: "bootstrap-token",
    targetDaemonId: "home-1",
  })
  const deps = createDeps({
    parseArgs: () => options,
    getRelayIdentity: () => identity,
    createClient: (_endpoint, relayOptions) => {
      createOptions.push(relayOptions ?? {})
      return createOptions.length === 1 ? initialClient : attachedClient
    },
    bootstrapAttachedSession: async (client, attachedOptions, workspace, worktree, preferences) => {
      assert.equal(client, attachedClient)
      return attachedBootstrap(client, attachedOptions, preferences)
    },
  })

  const result = await bootstrapCliRuntime({
    argv: ["chariox-terminal-pair-v1.fixture"],
    cwd: "/repo",
  }, deps)

  assert.equal(result.kind, "ready")
  assert.equal(closedInitialClient, true)
  assert.deepEqual(createOptions.map((entry) => entry.relayAuthToken), ["bootstrap-token", boundRelayToken])
  assert.equal(createOptions[0]?.relayIdentity, identity)
  assert.equal(createOptions[1]?.relayIdentity, identity)
  const join = capturedRequests[0] as { JoinTerminalPairingLink: { public_key_thumbprint: string } }
  assert.equal(join.JoinTerminalPairingLink.public_key_thumbprint, identity.publicKeyThumbprint)
})

test("terminal pairing closes bootstrap transport and rejects a legacy unbound token", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-bootstrap-legacy-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identity = createCliRelayIdentityStore(`${root}/relay/cli-identity-v1.json`).getOrCreate()
  let closeCount = 0
  let createCount = 0
  const initialClient = {
    async send<TResponse>() {
      return {
        TerminalPairingLinkJoined: {
          terminal: { terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 1, revoked: false },
          pairing: {
            intent: "client",
            subject_id: "terminal-1",
            relay_url: "wss://relay.example",
            target_daemon_id: "home-1",
            public_key_thumbprint: identity.publicKeyThumbprint,
            paired_at_ms: 1,
          },
        },
      } as TResponse
    },
    close: async () => { closeCount += 1 },
  } as LocalIpcClient
  const deps = createDeps({
    parseArgs: () => cliOptions({
      clientId: "terminal-1",
      relayUrl: "wss://relay.example",
      relayToken: "bootstrap-token",
      targetDaemonId: "home-1",
    }),
    getRelayIdentity: () => identity,
    createClient: () => {
      createCount += 1
      return initialClient
    },
  })

  await assert.rejects(
    bootstrapCliRuntime({ argv: ["chariox-terminal-pair-v1.fixture"], cwd: "/repo" }, deps),
    /requires a fresh protocol 349 relay token/,
  )
  assert.equal(createCount, 1)
  assert.equal(closeCount, 1)
})

test("buildDetachedBootstrap creates a waiting-room bootstrap shell", () => {
  const client = fakeClient()
  const options = cliOptions({ detached: true })
  const preferences: CharioxPreferences = {}

  const bootstrap = buildDetachedBootstrap(client, options, preferences)

  assert.equal(bootstrap.client, client)
  assert.equal(bootstrap.binding, null)
  assert.deepEqual(bootstrap.sessions, [])
  assert.equal(bootstrap.options, options)
  assert.equal(bootstrap.preferences, preferences)
})

function createDeps(overrides: Partial<CliRuntimeBootstrapDeps> = {}): CliRuntimeBootstrapDeps {
  const client = fakeClient()
  return {
    parseArgs: () => cliOptions(),
    loadPreferences: async () => ({}),
    applyProviderPreferenceDefaults: (options) => options,
    defaultKernelEndpoint: () => "ws://127.0.0.1:43118/kernel",
    createClient: () => client,
    getRelayIdentity: () => null,
    inferWorkspaceTargetsFromLaunchDirectory: async (cwd) => ({
      workspace: cwd,
      worktree: cwd,
    }),
    primeWaitingRoomWorktreeInventory: async () => {},
    loadThemeRegistry: async () => DEFAULT_THEME_REGISTRY,
    deleteSessionByRef: async () => {},
    isNoArgDefaultKernelLaunch: () => false,
    isKernelEndpointReachable: async () => true,
    isKernelEndpointUnavailableError: () => false,
    bootstrapAttachedSession: async (attachedClient, attachedOptions, _workspace, _worktree, preferences) =>
      attachedBootstrap(attachedClient, attachedOptions, preferences),
    maybeResize: async () => {},
    ...overrides,
  }
}

function cliOptions(overrides: Partial<CliOptions> = {}): CliOptions {
  return {
    clientId: "cli-1",
    provider: "opencode",
    model: "default",
    accountProfile: "default",
    effort: "",
    ...overrides,
  }
}

function fakeClient(): LocalIpcClient {
  return {} as LocalIpcClient
}

function attachedBootstrap(
  client: LocalIpcClient,
  options: CliOptions,
  preferences: CharioxPreferences,
): BootstrapState {
  return {
    client,
    binding: {
      session: { id: "session-1" },
      attachment: { id: "attachment-1" },
      providerRun: null,
      createdSession: true,
      historyEntries: [],
      promptHistoryEntries: [],
      nextHistoryCursor: null,
    },
    sessions: [],
    providerCatalog: {},
    providerCommandCatalogs: {},
    options,
    preferences,
  } as unknown as BootstrapState
}

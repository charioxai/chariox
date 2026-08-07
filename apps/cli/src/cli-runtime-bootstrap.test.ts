import { strict as assert } from "node:assert"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import type { BootstrapState, CliOptions } from "./cli-types.js"
import type { ArrobaPreferences } from "./preferences.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
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

test("buildDetachedBootstrap creates a waiting-room bootstrap shell", () => {
  const client = fakeClient()
  const options = cliOptions({ detached: true })
  const preferences: ArrobaPreferences = {}

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
  preferences: ArrobaPreferences,
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

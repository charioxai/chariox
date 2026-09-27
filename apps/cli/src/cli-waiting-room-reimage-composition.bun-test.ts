import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  ManagedEnvironmentContextPlan,
  ManagedEnvironmentReimagePreflight,
  ManagedEnvironmentReimageResult,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type {
  ProjectEnvironmentSetupStartInput,
} from "@chariox/kernel-client/project-environment-setup-orchestration"
import type {
  ProjectEnvironmentSetupStatus,
  RuntimeSession,
} from "@chariox/kernel-client/kernel-types"
import {
  createCliWaitingRoomComposition,
  type CliWaitingRoomCompositionDeps,
} from "./cli-waiting-room-composition.js"
import { LocalIpcClient } from "./ipc.js"
import {
  createMutableLocalIpcClient,
  type MutableLocalIpcClient,
} from "./mutable-local-ipc-client.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { managedEnvironmentMachineRef } from "./waiting-room-managed-environments.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

const LOCAL_ENDPOINT = "memory://local-kernel"
const OLD_ENDPOINT = "ws://old-kernel.test"
const REPLACEMENT_ENDPOINT = "ws://replacement-kernel.test"

function reimageTest(name: string, run: (router: TestRouter) => Promise<void>) {
  test(`production Waiting Room reimage composition: ${name}`, async () => {
  const router = installLocalIpcClientTestRouter()
  try {
    await run(router)
  } finally {
    router.restore()
  }
  })
}

reimageTest("observes the old kernel transactionally, rolls back failure, and requires confirmation", async (router) => {
      const harness = createHarness(router, { observationFailure: new Error("old observation failed") })
      try {
        await harness.initialize()
        await assert.rejects(
          harness.composition.reimageManagedEnvironment("environment-1", "prepare"),
          /old observation failed/,
        )

        assert.equal(harness.observedThroughEndpoint, OLD_ENDPOINT)
        assert.equal(harness.client.currentClient(), harness.local.client)
        assert.equal(harness.old.closeCount, 1)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
        await assert.rejects(
          harness.composition.reimageManagedEnvironment("environment-1", "confirm"),
          /Prepare this managed reimage/,
        )
      } finally {
        harness.cleanup()
      }
    })

reimageTest("keeps destructive admission local and launches the exact replacement identity", async (router) => {
      const harness = createHarness(router)
      try {
        await harness.initialize()
        const prepared = await harness.composition.reimageManagedEnvironment("environment-1", "prepare")

        assert.match(prepared.message, /Destructive confirmation required/)
        assert.match(prepared.message, /reimage environment-1 confirm/)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(harness.client.currentClient(), harness.local.client)

        const completed = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")

        assert.match(completed.message, /pivoted to the fresh kernel, and passed Project setup/)
        assert.equal(requestCount(harness.local, "RequestManagedEnvironmentReimage"), 1)
        assert.equal(requestCount(harness.old, "RequestManagedEnvironmentReimage"), 0)
        assert.equal(requestCount(harness.replacement, "RequestManagedEnvironmentReimage"), 0)
        assert.deepEqual(resolveTargets(harness.local), [
          { kernelRef: "kernel-old", machineRef: "machine-old" },
          { kernelRef: "kernel-new", machineRef: "machine-new" },
        ])
        assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
        assert.deepEqual(harness.attachments, [{ sessionId: "session-new", created: true }])
        assert.equal(harness.state().selectedMachineRef, managedEnvironmentMachineRef("environment-1"))
        const selectedReplacement = harness.composition.waitingRoomTargets()
          .managedEnvironmentCatalog?.environments[0]
        assert.equal(selectedReplacement?.runtimeMachineId, "machine-new")
        assert.equal(selectedReplacement?.runtimeKernelId, "kernel-new")
        assert.equal(harness.client.currentClient().socketPath, REPLACEMENT_ENDPOINT)
      } finally {
        harness.cleanup()
      }
    })

reimageTest("does not report cutover success when Project setup fails", async (router) => {
      const harness = createHarness(router, {
        contextPlan: sourcePlan(),
        setupFailure: new Error("dependency validation failed"),
      })
      try {
        await harness.initialize()
        await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
        let result: Awaited<ReturnType<typeof harness.composition.reimageManagedEnvironment>> | undefined

        await assert.rejects(async () => {
          result = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")
        }, /dependency validation failed/)

        assert.equal(result, undefined)
        assert.equal(requestCount(harness.replacement, "StartProjectEnvironmentSetup"), 1)
        assert.equal(requestCount(harness.replacement, "DeleteSession"), 1)
        assert.equal(harness.attachments.length, 0)
        assert.equal(harness.client.currentClient(), harness.local.client)
      } finally {
        harness.cleanup()
      }
    })

reimageTest("does not report cutover success when replacement attachment fails", async (router) => {
      const harness = createHarness(router, { attachFailure: new Error("attach rejected") })
      try {
        await harness.initialize()
        await harness.composition.reimageManagedEnvironment("environment-1", "prepare")
        let result: Awaited<ReturnType<typeof harness.composition.reimageManagedEnvironment>> | undefined

        await assert.rejects(async () => {
          result = await harness.composition.reimageManagedEnvironment("environment-1", "confirm")
        }, /attach rejected/)

        assert.equal(result, undefined)
        assert.equal(requestCount(harness.replacement, "CreateSession"), 1)
        assert.equal(requestCount(harness.replacement, "DeleteSession"), 1)
        assert.deepEqual(harness.rollbacks, ["session-new"])
        assert.equal(harness.client.currentClient(), harness.local.client)
      } finally {
        harness.cleanup()
      }
    })
type TestEndpoint = {
  readonly client: LocalIpcClient
  readonly requests: unknown[]
  closeCount: number
  send(request: unknown): Promise<unknown>
}

type TestRouter = ReturnType<typeof installLocalIpcClientTestRouter>

function installLocalIpcClientTestRouter() {
  const endpoints = new Map<string, TestEndpoint>()
  const send = LocalIpcClient.prototype.send
  const close = LocalIpcClient.prototype.close
  LocalIpcClient.prototype.send = function <TResponse>(request: unknown): Promise<TResponse> {
    const endpoint = endpoints.get(this.socketPath)
    if (!endpoint) {
      return Promise.reject(new Error(`unexpected LocalIpcClient endpoint ${this.socketPath}`))
    }
    endpoint.requests.push(structuredClone(request))
    return endpoint.send(request) as Promise<TResponse>
  }
  LocalIpcClient.prototype.close = async function (): Promise<void> {
    const endpoint = endpoints.get(this.socketPath)
    if (!endpoint) {
      throw new Error(`unexpected LocalIpcClient close for ${this.socketPath}`)
    }
    endpoint.closeCount += 1
  }
  return {
    endpoint(socketPath: string, responder: (request: unknown) => Promise<unknown> | unknown): TestEndpoint {
      const endpoint: TestEndpoint = {
        client: new LocalIpcClient(socketPath),
        requests: [],
        closeCount: 0,
        send: async (request) => await responder(request),
      }
      endpoints.set(socketPath, endpoint)
      return endpoint
    },
    restore() {
      LocalIpcClient.prototype.send = send
      LocalIpcClient.prototype.close = close
      endpoints.clear()
    },
  }
}

function createHarness(router: TestRouter, options: {
  observationFailure?: Error
  contextPlan?: ManagedEnvironmentContextPlan
  setupFailure?: Error
  attachFailure?: Error
} = {}) {
  const contextPlan = options.contextPlan ?? emptyPlan()
  const oldEnvironment = environment({
    desiredRevision: 7,
    observedRevision: 7,
    runtimeMachineId: "machine-old",
    runtimeKernelId: "kernel-old",
    runtimeReleaseDigest: "sha256:old-release",
    contextPlan,
  })
  const replacementEnvironment = environment({ contextPlan })
  const catalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: [],
    environments: [oldEnvironment],
  }
  let mutableClient!: MutableLocalIpcClient
  let observedThroughEndpoint: string | null = null
  const local = router.endpoint(LOCAL_ENDPOINT, async (request) => {
    switch (requestKind(request)) {
      case "GetWaitingRoomPublicSnapshot":
        return snapshotResponse("kernel-local", "machine-local")
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog } }
      case "GetManagedEnvironmentReimagePreflight":
        return { ManagedEnvironmentReimagePreflight: { preflight: preflight() } }
      case "ResolveKernelClientConnection": {
        const payload = requestPayload(request, "ResolveKernelClientConnection")
        const old = payload.kernel_ref === "kernel-old"
        return {
          KernelClientConnectionResolved: {
            connection: {
              relay_url: old ? OLD_ENDPOINT : REPLACEMENT_ENDPOINT,
              relay_token: old ? "old-token" : "replacement-token",
              target_daemon_id: old ? "kernel-old" : "kernel-new",
              target_daemon_alias: old ? "old" : "replacement",
              machine_id: old ? "machine-old" : "machine-new",
              kernel_id: old ? "kernel-old" : "kernel-new",
            },
          },
        }
      }
      case "RequestManagedEnvironmentReimage": {
        const payload = requestPayload(request, "RequestManagedEnvironmentReimage")
        return { ManagedEnvironmentReimageRequested: { result: reimageResult(replacementEnvironment, payload.idempotencyKey as string) } }
      }
      case "GetManagedEnvironment":
        return { ManagedEnvironment: { environment: replacementEnvironment } }
      default:
        throw new Error(`unexpected local request ${requestKind(request)}`)
    }
  })
  const old = router.endpoint(OLD_ENDPOINT, async (request) => {
    switch (requestKind(request)) {
      case "GetWaitingRoomPublicSnapshot":
        return snapshotResponse("kernel-old", "machine-old")
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog: { ...catalog, environments: [oldEnvironment] } } }
      case "ObserveManagedEnvironmentPreReimage":
        observedThroughEndpoint = mutableClient.currentClient().socketPath
        if (options.observationFailure) throw options.observationFailure
        return {
          ManagedEnvironmentPreReimageObserved: {
            acknowledgement: {
              environmentId: "environment-1",
              generation: 3,
              observedAt: "2026-09-22T00:00:00.000Z",
            },
          },
        }
      default:
        throw new Error(`unexpected old-kernel request ${requestKind(request)}`)
    }
  })
  const session = runtimeSession(contextPlan)
  const replacement = router.endpoint(REPLACEMENT_ENDPOINT, async (request) => {
    switch (requestKind(request)) {
      case "GetWaitingRoomPublicSnapshot":
        return snapshotResponse("kernel-new", "machine-new")
      case "ListSlices":
        return { SlicesListed: { slices: [] } }
      case "ListManagedEnvironmentCatalog":
        return { ManagedEnvironmentCatalog: { catalog: { ...catalog, environments: [replacementEnvironment] } } }
      case "GetManagedContextLaunchTarget":
        return { ManagedContextLaunchTarget: { target: launchTarget(contextPlan) } }
      case "CreateSession":
        return { SessionCreated: { session } }
      case "StartProjectEnvironmentSetup": {
        const input = requestPayload(request, "StartProjectEnvironmentSetup") as unknown as ProjectEnvironmentSetupStartInput
        return {
          ProjectEnvironmentSetupStarted: {
            status: setupStatus(input, options.setupFailure),
          },
        }
      }
      case "DeleteSession":
        return { SessionDeleted: { session } }
      default:
        throw new Error(`unexpected replacement-kernel request ${requestKind(request)}`)
    }
  })
  mutableClient = createMutableLocalIpcClient(local.client)

  const cacheDirectory = mkdtempSync(join(tmpdir(), "chariox-reimage-composition-"))
  const previousCacheDirectory = process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR
  process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR = cacheDirectory
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/staged",
    currentWorktreePath: "/staged/worktree",
    options: [{
      id: "existing:/staged/worktree",
      kind: "existing",
      label: "staged worktree",
      path: "/staged/worktree",
      branch: "test",
      isCurrent: true,
    }],
  })
  let waitingRoomState: WaitingRoomState = {
    ...createWaitingRoomState([], fallbackProviderCatalog(), "opencode", "opencode/gpt-5.4", "high"),
    worktreeSelectionId: "existing:/staged/worktree",
    selectedMachineRef: managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: "kernel-old",
  }
  let ownershipRevision = 0
  let relayStatus = relayStatusFor("kernel-local", "machine-local")
  let inventoryStatus: "loading" | "ready" | "error" = "ready"
  let availableSessions: unknown[] = []
  let projects: unknown[] = []
  let remoteMachines: unknown[] = []
  let remoteKernels: unknown[] = []
  let providerAccounts: unknown[] = []
  let terminals: unknown[] = []
  let slices: unknown[] = []
  let externalSessions: unknown[] = []
  let externalPage = { hasMore: false, nextCursor: null as string | null }
  let pendingWorkspace = ""
  let pendingWorktree = ""
  let providerCatalog = fallbackProviderCatalog()
  let preferences = {}
  const attachments: Array<{ sessionId: string; created: boolean }> = []
  const rollbacks: string[] = []
  const setValue = <T>(current: T, update: T | ((value: T) => T)): T => (
    typeof update === "function" ? (update as (value: T) => T)(current) : update
  )
  const noop = () => {}
  const deps: CliWaitingRoomCompositionDeps = {
    client: mutableClient,
    options: { clientId: "client-1", provider: "opencode", model: "opencode/gpt-5.4", effort: "high" },
    appLogger: { warn: noop, info: noop, debug: noop },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    isAttached: () => false,
    kernelConnected: () => true,
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (update: WaitingRoomState | ((value: WaitingRoomState) => WaitingRoomState)) => {
      waitingRoomState = setValue(waitingRoomState, update)
      ownershipRevision += 1
    },
    setWaitingRoomStateProjection: (next: WaitingRoomState) => { waitingRoomState = next },
    waitingRoomLaunchOwnershipRevision: () => ownershipRevision,
    availableSessions: () => availableSessions,
    setAvailableSessions: (update: unknown) => { availableSessions = setValue(availableSessions, update as never) },
    waitingRoomProjects: () => projects,
    setWaitingRoomProjects: (update: unknown) => { projects = setValue(projects, update as never) },
    providerCatalogState: () => providerCatalog,
    setProviderCatalogState: (next: typeof providerCatalog) => { providerCatalog = next },
    providerCommandCatalogState: () => ({}),
    setProviderCommandCatalogState: noop,
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => inventoryStatus,
    setWaitingRoomInventoryStatus: (next: typeof inventoryStatus) => { inventoryStatus = next },
    waitingRoomHiddenKernelController: { hideKernel: noop, isKernelHidden: () => false },
    relayStatusState: () => relayStatus,
    setRelayStatusState: (next: typeof relayStatus) => { relayStatus = next },
    remoteMachinesState: () => remoteMachines,
    setRemoteMachinesState: (update: unknown) => { remoteMachines = setValue(remoteMachines, update as never) },
    remoteKernelsState: () => remoteKernels,
    setRemoteKernelsState: (update: unknown) => { remoteKernels = setValue(remoteKernels, update as never) },
    providerAccountsState: () => providerAccounts,
    setProviderAccountsState: (update: unknown) => { providerAccounts = setValue(providerAccounts, update as never) },
    terminalsState: () => terminals,
    setTerminalsState: (update: unknown) => { terminals = setValue(terminals, update as never) },
    slicesState: () => slices,
    setSlicesState: (update: unknown) => { slices = setValue(slices, update as never) },
    externalProviderSessionsState: () => externalSessions,
    setExternalProviderSessionsState: (update: unknown) => { externalSessions = setValue(externalSessions, update as never) },
    externalProviderSessionsPageState: () => externalPage,
    setExternalProviderSessionsPageState: (next: typeof externalPage) => { externalPage = next },
    pendingWorkspaceTarget: () => pendingWorkspace,
    setPendingWorkspaceTarget: (next: string) => { pendingWorkspace = next },
    pendingWorktreeTarget: () => pendingWorktree,
    setPendingWorktreeTarget: (next: string) => { pendingWorktree = next },
    preferencesState: () => preferences,
    setPreferencesState: (update: unknown) => { preferences = setValue(preferences, update as never) },
    setThemeRevision: noop,
    resetTranscriptSyntax: noop,
    applyResponseLayout: noop,
    renderCommandCenter: noop,
    rebuildTranscript: noop,
    updateSessionChrome: noop,
    syncCommandCenter: noop,
    handleCloudCommand: async () => {},
    setPromptText: noop,
    focusPrompt: noop,
    openTerminalPairingDialog: async () => {},
    openSessionBrowserDialog: noop,
    attachBinding: async (attachedSession: RuntimeSession, created: boolean) => {
      if (options.attachFailure) throw options.attachFailure
      attachments.push({ sessionId: attachedSession.id, created })
    },
    rollbackAttachedSession: async (sessionId: string) => { rollbacks.push(sessionId) },
    flashFooter: noop,
    setKernelConnected: noop,
    setDaemonDisconnected: noop,
    sessionBrowserOpen: () => false,
    closeSessionBrowserDialog: noop,
    focusedProviderRun: () => null,
    focusedAgent: () => null,
    focusedAgentId: () => null,
    providerRunState: () => null,
    sessionState: () => null,
    applySessionState: noop,
    setProviderRunState: noop,
    appendNotice: noop,
  }
  const composition = createCliWaitingRoomComposition(deps)

  return {
    composition,
    initialize: () => composition.refreshWaitingRoomDataNow(),
    client: mutableClient,
    local,
    old,
    replacement,
    attachments,
    rollbacks,
    state: () => waitingRoomState,
    get observedThroughEndpoint() { return observedThroughEndpoint },
    cleanup() {
      __setWaitingRoomWorktreeInventoryForTest(null)
      if (previousCacheDirectory === undefined) {
        delete process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR
      } else {
        process.env.CHARIOX_WAITING_ROOM_INVENTORY_CACHE_DIR = previousCacheDirectory
      }
      rmSync(cacheDirectory, { recursive: true, force: true })
    },
  }
}

function requestKind(request: unknown): string {
  assert.ok(request && typeof request === "object" && !Array.isArray(request))
  const keys = Object.keys(request)
  assert.equal(keys.length, 1)
  return keys[0]!
}

function requestPayload(request: unknown, kind: string): Record<string, unknown> {
  assert.equal(requestKind(request), kind)
  return (request as Record<string, Record<string, unknown>>)[kind]!
}

function requestCount(endpoint: TestEndpoint, kind: string): number {
  return endpoint.requests.filter((request) => requestKind(request) === kind).length
}

function resolveTargets(endpoint: TestEndpoint) {
  return endpoint.requests
    .filter((request) => requestKind(request) === "ResolveKernelClientConnection")
    .map((request) => {
      const payload = requestPayload(request, "ResolveKernelClientConnection")
      return { kernelRef: payload.kernel_ref, machineRef: payload.machine_ref }
    })
}

function snapshotResponse(kernelId: string, machineId: string) {
  return {
    WaitingRoomPublicSnapshot: {
      snapshot: {
        schema_version: 11,
        inventory_version: `${kernelId}:inventory`,
        structural_version: `${kernelId}:structural`,
        activity_revision: `${kernelId}:activity`,
        generated_at_ms: 1,
        sessions: [],
        projects: [],
        relay_status: relayStatusFor(kernelId, machineId),
        remote_machines: [],
        remote_kernels: [],
        terminals: [],
        provider_accounts: [],
        git_credentials: [],
      },
    },
  }
}

function relayStatusFor(kernelId: string, machineId: string) {
  return {
    configured: true,
    connected: true,
    relay_url: "wss://relay.test",
    daemon_id: kernelId,
    daemon_alias: kernelId,
    machine_id: machineId,
    machine_alias: machineId,
  }
}

function preflight(): ManagedEnvironmentReimagePreflight {
  return {
    environmentId: "environment-1",
    retained: {
      providerServerId: "server-1",
      generation: 3,
      desiredRevision: 7,
      observedRevision: 7,
      runtimeMachineId: "machine-old",
      runtimeKernelId: "kernel-old",
      runtimeRelayRealmId: "realm-old",
      runtimeReleaseDigest: "sha256:old-release",
    },
    desiredRelease: {
      providerId: "hetzner",
      providerImageId: "image-2",
      providerProfileId: "path1",
      providerProfileDigest: "sha256:profile",
      runtimeReleaseDigest: "sha256:new-release",
      runtimeSourceCommit: "1".repeat(40),
      runtimeSourceTree: "2".repeat(40),
    },
  }
}

function emptyPlan(): ManagedEnvironmentContextPlan {
  return {
    schemaVersion: 1,
    contextId: "context-new",
    planDigest: "sha256:context",
    source: null,
    kernelContext: "empty",
    developmentSetup: { kind: "empty" },
    providerAccounts: { kind: "none" },
    gitCredentials: { kind: "none" },
  }
}

function sourcePlan(): ManagedEnvironmentContextPlan {
  return {
    ...emptyPlan(),
    source: {
      sourceTargetId: "source-target-1",
      relayRealmId: "realm-source",
      machineId: "machine-source",
      kernelId: "kernel-source",
      keyThumbprint: "sha256:source-key",
    },
    kernelContext: "source_kernel",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [{ role: "primary", workspaceId: "workspace-primary", worktreeId: "worktree-primary" }],
    },
  }
}

function environment(overrides: Partial<ManagedEnvironmentSummary> = {}): ManagedEnvironmentSummary {
  return {
    environmentId: "environment-1",
    accountId: "account-1",
    createdByUserId: "user-1",
    name: "Managed agent",
    region: "hel1",
    computeClass: "agent-small",
    managedRepositoryRoot: "/home/chariox",
    desiredState: "running",
    observedState: "ready",
    desiredRevision: 8,
    observedRevision: 8,
    runtimeMachineId: "machine-new",
    runtimeKernelId: "kernel-new",
    runtimeReleaseDigest: "sha256:new-release",
    contextPlan: emptyPlan(),
    contextManifestDigest: "sha256:manifest",
    autoStopPolicy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 },
    lastErrorCode: null,
    lastErrorMessage: null,
    createdAt: "2026-09-22T00:00:00.000Z",
    updatedAt: "2026-09-22T00:01:00.000Z",
    ...overrides,
  }
}

function reimageResult(
  replacement: ManagedEnvironmentSummary,
  idempotencyKey: string,
): ManagedEnvironmentReimageResult {
  return {
    environment: replacement,
    operation: {
      operationId: "operation-1",
      environmentId: "environment-1",
      requestedByUserId: "user-1",
      kind: "reimage",
      idempotencyKey,
      requestDigest: "sha256:request",
      desiredRevision: 8,
      status: "succeeded",
      attempt: 1,
      retryable: false,
      failureCode: null,
      failureMessage: null,
      completedAt: "2026-09-22T00:01:00.000Z",
      createdAt: "2026-09-22T00:00:00.000Z",
      updatedAt: "2026-09-22T00:01:00.000Z",
    },
    receipt: {
      receiptId: "receipt-1",
      environmentId: "environment-1",
      operationId: "operation-1",
      previousGeneration: 3,
      generation: 4,
      status: "fresh_equivalent",
      freshEquivalent: true,
      providerServerId: "server-1",
      previousProviderImageId: "image-1",
      providerImageId: "image-2",
      providerProfileId: "path1",
      providerProfileDigest: "sha256:profile",
      runtimeReleaseDigest: "sha256:new-release",
      oldMachineId: "machine-old",
      newMachineId: "machine-new",
      oldKernelId: "kernel-old",
      newKernelId: "kernel-new",
      oldRelayRealmId: "realm-old",
      newRelayRealmId: "realm-new",
      oldRelayTargetId: "target-old",
      newRelayTargetId: "target-new",
      oldBootstrapGrantId: "grant-old",
      newBootstrapGrantId: "grant-new",
      oldCredentialIds: [],
      newCredentialIds: [],
      runtimeEvidence: {},
      sourceEvidence: {
        providerImageId: "image-2",
        providerProfileId: "path1",
        providerProfileDigest: "sha256:profile",
        runtimeReleaseDigest: "sha256:new-release",
        runtimeSourceCommit: "1".repeat(40),
        runtimeSourceTree: "2".repeat(40),
      },
      residueChecks: {},
      revocations: {},
      billingObservation: {},
      resourceObservation: {},
      cleanupState: {},
      rollbackState: {},
      receiptDigest: "sha256:receipt",
      failureCode: null,
      failureMessage: null,
      requestedAt: "2026-09-22T00:00:00.000Z",
      completedAt: "2026-09-22T00:01:00.000Z",
      createdAt: "2026-09-22T00:00:00.000Z",
      updatedAt: "2026-09-22T00:01:00.000Z",
    },
  }
}

function launchTarget(plan: ManagedEnvironmentContextPlan) {
  return {
    environmentId: "environment-1",
    kernelId: "kernel-new",
    contextId: plan.contextId,
    planDigest: plan.planDigest,
    development: plan.developmentSetup.kind === "empty"
      ? { kind: "empty" as const, workspacePath: "/managed/empty/context-new" }
      : {
          kind: "from_source" as const,
          projectId: plan.developmentSetup.projectId,
          destinationRoot: "/managed/context-new",
          primaryRepositoryId: "repository-primary",
          repositories: [{
            repositoryId: "repository-primary",
            role: "primary" as const,
            targetDirectory: "primary",
            workspacePath: "/managed/context-new/primary",
            headSha: "a".repeat(40),
          }],
        },
  }
}

function runtimeSession(plan: ManagedEnvironmentContextPlan): RuntimeSession {
  const workspace = plan.developmentSetup.kind === "empty"
    ? "/managed/empty/context-new"
    : "/managed/context-new/primary"
  return {
    id: "session-new",
    project_id: plan.developmentSetup.kind === "empty" ? "default" : plan.developmentSetup.projectId,
    workspace_id: workspace,
    worktree_id: workspace,
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [{ id: "agent-1" } as RuntimeSession["agents"][number]],
    config_state: { version: 1, values: {}, updated_by_attachment_id: null },
  }
}

function setupStatus(
  input: ProjectEnvironmentSetupStartInput,
  failure?: Error,
): ProjectEnvironmentSetupStatus {
  return {
    operation_id: input.operationId,
    project_id: input.projectId,
    session_id: input.sessionId,
    agent_id: input.agentId,
    worker_id: input.targetWorkerId,
    platform: input.targetPlatform,
    phase: failure ? "failed" : "ready",
    attempt: 1,
    progress_percent: failure ? 60 : 100,
    definition_digest: "sha256:definition",
    validation: {
      worker_id: input.targetWorkerId,
      platform: input.targetPlatform,
      commands: [],
    },
    message: failure?.message ?? "ready",
    failure_code: failure ? "validation_failed" : null,
    failure_message: failure?.message ?? null,
    retryable: false,
    created_at_ms: 1,
    updated_at_ms: 2,
  }
}

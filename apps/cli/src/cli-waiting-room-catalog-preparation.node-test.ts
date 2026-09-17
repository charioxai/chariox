// Run separately from the normal Node suite: composition imports Bun-only UI bindings.
import assert from "node:assert/strict"
import test from "node:test"

import { createCliWaitingRoomComposition } from "./cli-waiting-room-composition.js"
import { LocalIpcClient } from "./ipc.js"
import { createMutableLocalIpcClient } from "./mutable-local-ipc-client.js"
import { type ProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

test("waiting room start awaits a failed account catalog refresh for an already-connected owner", async () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [{
      id: "existing:/workspace",
      kind: "existing",
      label: "main",
      path: "/workspace",
      branch: "main",
      isCurrent: true,
    }],
  })

  try {
    const defaultCatalog = catalog("Default account model")
    const workCatalog = catalog("Work account model")
    const defaultAccount = account("default", "Default", true)
    const workAccount = account("work", "Work", false)
    let providerCatalog = defaultCatalog
    let waitingRoomState = {
      ...createWaitingRoomState(
        [],
        defaultCatalog,
        "opencode",
        "opencode/shared-model",
        "",
      ),
      selectedMachineRef: "machine-current",
      selectedKernelRef: "kernel-current",
    }
    let catalogRequests = 0
    let resolveCatalogRefreshStarted: (() => void) | undefined
    let rejectCatalogRefresh: ((error: Error) => void) | undefined
    const catalogRefreshStarted = new Promise<void>((resolve) => {
      resolveCatalogRefreshStarted = resolve
    })
    const pendingCatalogRefresh = new Promise<never>((_, reject) => {
      rejectCatalogRefresh = reject
    })
    let createdSessions = 0
    const createdLaunches: unknown[] = []
    const options: Record<string, unknown> = {
      provider: "opencode",
      accountProfile: "default",
      model: "opencode/shared-model",
      effort: "",
      clientId: "composition-p1-test",
    }
    const client = {
      send: async (request: unknown) => {
        if (typeof request === "object" && request !== null && "GetProviderCatalog" in request) {
          catalogRequests += 1
          if (catalogRequests === 1) {
            return { ProviderCatalog: { catalog: workCatalog } }
          }
          resolveCatalogRefreshStarted?.()
          return pendingCatalogRefresh
        }
        if (typeof request === "object" && request !== null && "CreateSession" in request) {
          createdSessions += 1
          createdLaunches.push(request)
          return {
            SessionCreated: {
              session: {
                id: "created-session",
                project_id: "project-default",
                alias: null,
                workspace_id: "/workspace",
                worktree_id: "/workspace",
                created_at_ms: 1,
                status: "Active",
                agent_defaults: {
                  provider: "opencode",
                  model: "opencode/shared-model",
                  effort: "",
                  account_profile: "work",
                  execution_mode: "build",
                  permission_level: "yolo",
                },
                active_provider_run_id: null,
                attachment_ids: [],
                active_prompt: null,
                queued_prompts: [],
                focused_agent_id: null,
                max_agents: 1,
                agents: [],
                workflows: [],
                workflow_runs: [],
                workflow_watchdogs: [],
                workflow_consoles: [],
                config_state: {
                  version: 1,
                  values: {},
                  updated_by_attachment_id: null,
                },
              },
            },
          }
        }
        throw new Error("unexpected IPC request in composition regression")
      },
    }
    const providerAccounts = [defaultAccount, workAccount]
    const relay = {
      configured: true,
      connected: true,
      daemon_id: "kernel-current",
      machine_id: "machine-current",
    }
    const composition = createCliWaitingRoomComposition({
      client,
      options,
      appLogger: { info: () => {}, warn: () => {} },
      formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
      isAttached: () => false,
      kernelConnected: () => true,
      waitingRoomState: () => waitingRoomState,
      setWaitingRoomState: (next: typeof waitingRoomState) => {
        waitingRoomState = next
      },
      setWaitingRoomStateProjection: (next: typeof waitingRoomState) => {
        waitingRoomState = next
      },
      waitingRoomLaunchOwnershipRevision: () => 0,
      availableSessions: () => [],
      setAvailableSessions: () => {},
      waitingRoomProjects: () => [],
      setWaitingRoomProjects: () => {},
      providerCatalogState: () => providerCatalog,
      setProviderCatalogState: (next: ProviderCatalog) => {
        providerCatalog = next
      },
      providerCommandCatalogState: () => ({}),
      setProviderCommandCatalogState: () => {},
      themeRegistryState: () => DEFAULT_THEME_REGISTRY,
      waitingRoomCloudNotice: () => null,
      waitingRoomInventoryStatus: () => "ready",
      setWaitingRoomInventoryStatus: () => {},
      waitingRoomHiddenKernelController: {
        hideKernel: () => {},
        isKernelHidden: () => false,
      },
      relayStatusState: () => relay,
      setRelayStatusState: () => {},
      remoteMachinesState: () => [{
        machine_id: "machine-current",
        kernel_count: 1,
        available_providers: ["opencode"],
      }],
      setRemoteMachinesState: () => {},
      remoteKernelsState: () => [{
        kernel_id: "kernel-current",
        machine_id: "machine-current",
        available_providers: ["opencode"],
      }],
      setRemoteKernelsState: () => {},
      providerAccountsState: () => providerAccounts,
      setProviderAccountsState: () => {},
      terminalsState: () => [],
      setTerminalsState: () => {},
      slicesState: () => [],
      setSlicesState: () => {},
      externalProviderSessionsState: () => [],
      setExternalProviderSessionsState: () => {},
      externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
      setExternalProviderSessionsPageState: () => {},
      pendingWorkspaceTarget: () => "/workspace",
      setPendingWorkspaceTarget: () => {},
      pendingWorktreeTarget: () => "/workspace",
      setPendingWorktreeTarget: () => {},
      preferencesState: () => ({}),
      setPreferencesState: () => {},
      setThemeRevision: () => {},
      resetTranscriptSyntax: () => {},
      applyResponseLayout: () => {},
      renderCommandCenter: () => {},
      rebuildTranscript: () => {},
      updateSessionChrome: () => {},
      syncCommandCenter: () => {},
      handleCloudCommand: async () => {},
      setPromptText: () => {},
      focusPrompt: () => {},
      openTerminalPairingDialog: async () => {},
      openSessionBrowserDialog: () => {},
      closeSessionBrowserDialog: () => {},
      attachBinding: async () => {},
      rollbackAttachedSession: async () => {},
      flashFooter: () => {},
      setKernelConnected: () => {},
      setDaemonDisconnected: () => {},
      sessionBrowserOpen: () => false,
      focusedProviderRun: () => null,
      focusedAgent: () => null,
      focusedAgentId: () => null,
      providerRunState: () => null,
      sessionState: () => ({ id: "session-1" }),
      applySessionState: () => {},
      setProviderRunState: () => {},
      appendNotice: () => {},
    } as never)

    await composition.applyAccountSelection("Work")
    await catalogRefreshStarted
    assert.ok(catalogRequests >= 2, "account selection must start a detached catalog refresh")

    let startSettled = false
    const startPromise = composition.startSessionFromWaitingRoomDefaults()
    void startPromise.then(
      () => { startSettled = true },
      () => { startSettled = true },
    )
    await new Promise<void>((resolve) => setImmediate(resolve))
    assert.equal(startSettled, false, "Start must await the selected account catalog for the current owner")
    assert.equal(createdSessions, 0, "Start must not create a session before catalog discovery completes")

    rejectCatalogRefresh?.(new Error("selected account catalog unavailable"))
    await assert.rejects(startPromise, /selected account catalog unavailable/)
    assert.equal(createdSessions, 0, "Start must abort before creating a session when discovery fails")
    assert.deepEqual(createdLaunches, [])
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room start aborts when the selected kernel changes during target preparation", async () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [{
      id: "existing:/workspace",
      kind: "existing",
      label: "main",
      path: "/workspace",
      branch: "main",
      isCurrent: true,
    }],
  })

  const originalSend = LocalIpcClient.prototype.send
  const originalClose = LocalIpcClient.prototype.close
  const sourceClient = new LocalIpcClient("source")
  let sourceCloseCount = 0
  let targetCloseCount = 0
  let targetCatalogRequests = 0
  const createdSessionOwners: string[] = []
  let resolveInventoryStarted: (() => void) | undefined
  let resolveCatalogStarted: (() => void) | undefined
  let releaseInventory: (() => void) | undefined
  let releaseCatalog: (() => void) | undefined
  const inventoryStarted = new Promise<void>((resolve) => {
    resolveInventoryStarted = resolve
  })
  const catalogStarted = new Promise<void>((resolve) => {
    resolveCatalogStarted = resolve
  })
  const inventoryGate = new Promise<void>((resolve) => {
    releaseInventory = resolve
  })
  const catalogGate = new Promise<void>((resolve) => {
    releaseCatalog = resolve
  })
  let startPromise: Promise<unknown> | undefined

  ;(sourceClient as any).close = async () => {
    sourceCloseCount += 1
  }
  ;(sourceClient as any).send = async (request: unknown) => {
    if (hasRequest(request, "ResolveKernelClientConnection")) {
      return {
        KernelClientConnectionResolved: {
          connection: {
            relay_url: "ws://kernel-old",
            relay_token: "relay-token",
            target_daemon_id: "kernel-old",
            target_daemon_alias: null,
            machine_id: "machine-old",
            kernel_id: "kernel-old",
            token_expires_at: null,
          },
        },
      }
    }
    if (hasRequest(request, "GetProviderCatalog")) {
      return { ProviderCatalog: { catalog: catalog("new-kernel") } }
    }
    if (hasRequest(request, "CreateSession")) {
      createdSessionOwners.push("source")
      return createdSessionResponse()
    }
    throw new Error(`unexpected source IPC request: ${requestKind(request)}`)
  }
  ;(LocalIpcClient.prototype as any).send = async function (request: unknown) {
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "GetWaitingRoomPublicSnapshot")) {
      resolveInventoryStarted?.()
      await inventoryGate
      return { WaitingRoomPublicSnapshot: { snapshot: publicSnapshot("kernel-old", "machine-old") } }
    }
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "GetProviderCatalog")) {
      targetCatalogRequests += 1
      resolveCatalogStarted?.()
      await catalogGate
      return { ProviderCatalog: { catalog: catalog("old-kernel") } }
    }
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "CreateSession")) {
      createdSessionOwners.push("kernel-old")
      return createdSessionResponse()
    }
    throw new Error(`unexpected target IPC request: ${requestKind(request)}`)
  }
  ;(LocalIpcClient.prototype as any).close = async function () {
    if (this.socketPath === "ws://kernel-old") {
      targetCloseCount += 1
    }
    await originalClose.call(this)
  }

  const client = createMutableLocalIpcClient(sourceClient)
  let waitingRoomState = {
    ...createWaitingRoomState(
      [],
      catalog("source-kernel"),
      "opencode",
      "opencode/shared-model",
      "",
    ),
    selectedMachineRef: "machine-old",
    selectedKernelRef: "kernel-old",
  }
  let providerCatalog = catalog("source-kernel")
  const providerAccounts = [account("default", "Default", true)]
  const relay = {
    configured: true,
    connected: true,
    daemon_id: "kernel-source",
    machine_id: "machine-source",
  }
  const options: Record<string, unknown> = {
    provider: "opencode",
    accountProfile: "default",
    model: "opencode/shared-model",
    effort: "",
    clientId: "composition-kernel-selection-race-test",
  }

  try {
    const composition = createCliWaitingRoomComposition({
      client,
      options,
      appLogger: { info: () => {}, warn: () => {} },
      formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
      isAttached: () => false,
      kernelConnected: () => true,
      waitingRoomState: () => waitingRoomState,
      setWaitingRoomState: (next: typeof waitingRoomState) => {
        waitingRoomState = next
      },
      setWaitingRoomStateProjection: (next: typeof waitingRoomState) => {
        waitingRoomState = next
      },
      waitingRoomLaunchOwnershipRevision: () => 0,
      availableSessions: () => [],
      setAvailableSessions: () => {},
      waitingRoomProjects: () => [],
      setWaitingRoomProjects: () => {},
      providerCatalogState: () => providerCatalog,
      setProviderCatalogState: (next: ProviderCatalog) => {
        providerCatalog = next
      },
      providerCommandCatalogState: () => ({}),
      setProviderCommandCatalogState: () => {},
      themeRegistryState: () => DEFAULT_THEME_REGISTRY,
      waitingRoomCloudNotice: () => null,
      waitingRoomInventoryStatus: () => "ready",
      setWaitingRoomInventoryStatus: () => {},
      waitingRoomHiddenKernelController: {
        hideKernel: () => {},
        isKernelHidden: () => false,
      },
      relayStatusState: () => relay,
      setRelayStatusState: () => {},
      remoteMachinesState: () => [
        { machine_id: "machine-old", kernel_count: 1, available_providers: ["opencode"] },
        { machine_id: "machine-new", kernel_count: 1, available_providers: ["opencode"] },
      ],
      setRemoteMachinesState: () => {},
      remoteKernelsState: () => [
        { kernel_id: "kernel-old", machine_id: "machine-old", available_providers: ["opencode"] },
        { kernel_id: "kernel-new", machine_id: "machine-new", available_providers: ["opencode"] },
      ],
      setRemoteKernelsState: () => {},
      providerAccountsState: () => providerAccounts,
      setProviderAccountsState: () => {},
      terminalsState: () => [],
      setTerminalsState: () => {},
      slicesState: () => [],
      setSlicesState: () => {},
      externalProviderSessionsState: () => [],
      setExternalProviderSessionsState: () => {},
      externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
      setExternalProviderSessionsPageState: () => {},
      pendingWorkspaceTarget: () => "/workspace",
      setPendingWorkspaceTarget: () => {},
      pendingWorktreeTarget: () => "/workspace",
      setPendingWorktreeTarget: () => {},
      preferencesState: () => ({}),
      setPreferencesState: () => {},
      setThemeRevision: () => {},
      resetTranscriptSyntax: () => {},
      applyResponseLayout: () => {},
      renderCommandCenter: () => {},
      rebuildTranscript: () => {},
      updateSessionChrome: () => {},
      syncCommandCenter: () => {},
      handleCloudCommand: async () => {},
      setPromptText: () => {},
      focusPrompt: () => {},
      openTerminalPairingDialog: async () => {},
      openSessionBrowserDialog: () => {},
      closeSessionBrowserDialog: () => {},
      attachBinding: async () => {},
      rollbackAttachedSession: async () => {},
      flashFooter: () => {},
      setKernelConnected: () => {},
      setDaemonDisconnected: () => {},
      sessionBrowserOpen: () => false,
      focusedProviderRun: () => null,
      focusedAgent: () => null,
      focusedAgentId: () => null,
      providerRunState: () => null,
      sessionState: () => ({ id: "session-1" }),
      applySessionState: () => {},
      setProviderRunState: () => {},
      appendNotice: () => {},
    } as never)

    startPromise = composition.startSessionFromWaitingRoomDefaults()
    await inventoryStarted
    assert.equal(client.currentClient(), sourceClient, "the source client stays active while target I/O is deferred")

    composition.reconcileWaitingRoom({
      ...waitingRoomState,
      selectedMachineRef: "machine-new",
      selectedKernelRef: "kernel-new",
    })
    assert.equal(waitingRoomState.selectedKernelRef, "kernel-new")
    assert.equal(waitingRoomState.selectedMachineRef, "machine-new")

    releaseInventory?.()
    await catalogStarted
    assert.equal(targetCatalogRequests, 1)
    assert.equal(client.currentClient(), sourceClient, "selection pivot must not commit before target catalog completes")
    releaseCatalog?.()

    await assert.rejects(startPromise, /selection changed/)
    assert.deepEqual(createdSessionOwners, [], "selection races must not create on either kernel owner")
    assert.equal(client.currentClient(), sourceClient, "the failed pivot must preserve the original client")
    assert.equal(sourceCloseCount, 0, "the source client must not be closed by an abandoned pivot")
    assert.ok(targetCloseCount >= 1, "an abandoned target client must be rolled back")
  } finally {
    releaseInventory?.()
    releaseCatalog?.()
    await startPromise?.catch(() => {})
    ;(LocalIpcClient.prototype as any).send = originalSend
    ;(LocalIpcClient.prototype as any).close = originalClose
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room start completes an unchanged remote pivot through the retained client transaction", async () => {
  const fixture = createRemoteCompositionFixture()
  let startPromise: Promise<unknown> | undefined
  try {
    startPromise = fixture.composition.startSessionFromWaitingRoomDefaults()
    await fixture.targetCatalogStarted
    assert.equal(fixture.client.currentClient().socketPath, "ws://kernel-old")

    fixture.releaseTargetCatalog()
    const session = await startPromise
    assert.equal((session as { id: string }).id, "created-session")
    assert.deepEqual(fixture.createdSessionOwners, ["kernel-old"])
    assert.equal(fixture.client.currentClient().socketPath, "ws://kernel-old")
    assert.equal(fixture.sourceCloseCount(), 1, "a successful retained pivot commits the source close")
    assert.equal(fixture.targetCloseCount(), 0)
  } finally {
    fixture.releaseTargetCatalog()
    await startPromise?.catch(() => {})
    await fixture.cleanup()
  }
})

test("waiting room start rolls back a post-pivot away-and-back selection change", async () => {
  const fixture = createRemoteCompositionFixture()
  let startPromise: Promise<unknown> | undefined
  try {
    startPromise = fixture.composition.startSessionFromWaitingRoomDefaults()
    await fixture.targetCatalogStarted
    assert.equal(fixture.client.currentClient().socketPath, "ws://kernel-old")

    fixture.composition.reconcileWaitingRoom({
      ...fixture.waitingRoomState(),
      selectedMachineRef: "machine-new",
      selectedKernelRef: "kernel-new",
    })
    fixture.composition.reconcileWaitingRoom({
      ...fixture.waitingRoomState(),
      selectedMachineRef: "machine-old",
      selectedKernelRef: "kernel-old",
    })
    fixture.releaseTargetCatalog()

    await assert.rejects(startPromise, /selection changed/)
    assert.deepEqual(fixture.createdSessionOwners, [])
    assert.equal(fixture.client.currentClient(), fixture.sourceClient)
    assert.equal(fixture.sourceCloseCount(), 0, "away-and-back cancellation preserves the source client")
    assert.ok(fixture.targetCloseCount() >= 1, "away-and-back cancellation rolls back the target client")
  } finally {
    fixture.releaseTargetCatalog()
    await startPromise?.catch(() => {})
    await fixture.cleanup()
  }
})

test("waiting room start cleans up a session when selection changes during CreateSession", async () => {
  const fixture = createRemoteCompositionFixture({ deferCreateSession: true })
  let startPromise: Promise<unknown> | undefined
  try {
    startPromise = fixture.composition.startSessionFromWaitingRoomDefaults()
    await fixture.targetCatalogStarted
    fixture.releaseTargetCatalog()
    await fixture.createSessionStarted

    fixture.composition.reconcileWaitingRoom({
      ...fixture.waitingRoomState(),
      selectedMachineRef: "machine-new",
      selectedKernelRef: "kernel-new",
    })
    fixture.releaseCreateSession()

    await assert.rejects(startPromise, /selection changed/)
    assert.deepEqual(fixture.createdSessionOwners, ["kernel-old"])
    assert.deepEqual(fixture.deletedSessionIds, ["created-session"])
    assert.deepEqual(fixture.attachedSessionIds, [])
    assert.deepEqual(fixture.rolledBackAttachmentIds, [])
    assert.equal(fixture.client.currentClient(), fixture.sourceClient)
    assert.equal(fixture.sourceCloseCount(), 0)
    assert.ok(fixture.targetCloseCount() >= 1)
  } finally {
    fixture.releaseTargetCatalog()
    fixture.releaseCreateSession()
    fixture.releaseAttachBinding()
    await startPromise?.catch(() => {})
    await fixture.cleanup()
  }
})

test("waiting room start rolls back a created and attached session when selection changes during attachBinding", async () => {
  const fixture = createRemoteCompositionFixture({ deferAttachBinding: true })
  let startPromise: Promise<unknown> | undefined
  try {
    startPromise = fixture.composition.startSessionFromWaitingRoomDefaults()
    await fixture.targetCatalogStarted
    fixture.releaseTargetCatalog()
    await fixture.attachBindingStarted

    fixture.composition.reconcileWaitingRoom({
      ...fixture.waitingRoomState(),
      selectedMachineRef: "machine-new",
      selectedKernelRef: "kernel-new",
    })
    fixture.releaseAttachBinding()

    await assert.rejects(startPromise, /selection changed/)
    assert.deepEqual(fixture.createdSessionOwners, ["kernel-old"])
    assert.deepEqual(fixture.attachedSessionIds, ["created-session"])
    assert.deepEqual(fixture.rolledBackAttachmentIds, ["created-session"])
    assert.deepEqual(fixture.deletedSessionIds, ["created-session"])
    assert.equal(fixture.client.currentClient(), fixture.sourceClient)
    assert.equal(fixture.sourceCloseCount(), 0)
    assert.ok(fixture.targetCloseCount() >= 1)
  } finally {
    fixture.releaseTargetCatalog()
    fixture.releaseCreateSession()
    fixture.releaseAttachBinding()
    await startPromise?.catch(() => {})
    await fixture.cleanup()
  }
})

function createRemoteCompositionFixture(fixtureOptions: {
  deferCreateSession?: boolean
  deferAttachBinding?: boolean
} = {}) {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [{
      id: "existing:/workspace",
      kind: "existing",
      label: "main",
      path: "/workspace",
      branch: "main",
      isCurrent: true,
    }],
  })

  const originalSend = LocalIpcClient.prototype.send
  const originalClose = LocalIpcClient.prototype.close
  const sourceClient = new LocalIpcClient("source")
  let sourceCloseCount = 0
  let targetCloseCount = 0
  let targetCatalogRequests = 0
  let resolveTargetCatalogStarted: (() => void) | undefined
  let releaseTargetCatalog: (() => void) | undefined
  let resolveCreateSessionStarted: (() => void) | undefined
  let releaseCreateSession: (() => void) | undefined
  let resolveAttachBindingStarted: (() => void) | undefined
  let releaseAttachBinding: (() => void) | undefined
  const targetCatalogStarted = new Promise<void>((resolve) => {
    resolveTargetCatalogStarted = resolve
  })
  const targetCatalogGate = new Promise<void>((resolve) => {
    releaseTargetCatalog = resolve
  })
  const createSessionStarted = new Promise<void>((resolve) => {
    resolveCreateSessionStarted = resolve
  })
  const createSessionGate = new Promise<void>((resolve) => {
    releaseCreateSession = resolve
  })
  const attachBindingStarted = new Promise<void>((resolve) => {
    resolveAttachBindingStarted = resolve
  })
  const attachBindingGate = new Promise<void>((resolve) => {
    releaseAttachBinding = resolve
  })
  const createdSessionOwners: string[] = []
  const deletedSessionIds: string[] = []
  const attachedSessionIds: string[] = []
  const rolledBackAttachmentIds: string[] = []

  ;(sourceClient as any).close = async () => {
    sourceCloseCount += 1
  }
  ;(sourceClient as any).send = async (request: unknown) => {
    if (hasRequest(request, "ResolveKernelClientConnection")) {
      return {
        KernelClientConnectionResolved: {
          connection: {
            relay_url: "ws://kernel-old",
            relay_token: "relay-token",
            target_daemon_id: "kernel-old",
            target_daemon_alias: null,
            machine_id: "machine-old",
            kernel_id: "kernel-old",
            token_expires_at: null,
          },
        },
      }
    }
    if (hasRequest(request, "GetProviderCatalog")) {
      return { ProviderCatalog: { catalog: catalog("source-kernel") } }
    }
    if (hasRequest(request, "CreateSession")) {
      createdSessionOwners.push("source")
      return createdSessionResponse()
    }
    throw new Error(`unexpected source IPC request: ${requestKind(request)}`)
  }
  ;(LocalIpcClient.prototype as any).send = async function (request: unknown) {
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "GetWaitingRoomPublicSnapshot")) {
      return { WaitingRoomPublicSnapshot: { snapshot: publicSnapshot("kernel-old", "machine-old") } }
    }
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "GetProviderCatalog")) {
      targetCatalogRequests += 1
      if (targetCatalogRequests === 2) {
        resolveTargetCatalogStarted?.()
        await targetCatalogGate
      }
      return { ProviderCatalog: { catalog: catalog("old-kernel") } }
    }
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "CreateSession")) {
      resolveCreateSessionStarted?.()
      if (fixtureOptions.deferCreateSession) {
        await createSessionGate
      }
      createdSessionOwners.push("kernel-old")
      return createdSessionResponse()
    }
    if (this.socketPath === "ws://kernel-old" && hasRequest(request, "DeleteSession")) {
      const payload = (request as { DeleteSession?: { session_ref?: string } }).DeleteSession
      if (payload?.session_ref) {
        deletedSessionIds.push(payload.session_ref)
      }
      return deletedSessionResponse()
    }
    throw new Error(`unexpected target IPC request: ${requestKind(request)}`)
  }
  ;(LocalIpcClient.prototype as any).close = async function () {
    if (this.socketPath === "ws://kernel-old") {
      targetCloseCount += 1
    }
    await originalClose.call(this)
  }

  const client = createMutableLocalIpcClient(sourceClient)
  let waitingRoomState = {
    ...createWaitingRoomState(
      [],
      catalog("source-kernel"),
      "opencode",
      "opencode/shared-model",
      "",
    ),
    selectedMachineRef: "machine-old",
    selectedKernelRef: "kernel-old",
  }
  let providerCatalog = catalog("source-kernel")
  const providerAccounts = [account("default", "Default", true)]
  const relay = {
    configured: true,
    connected: true,
    daemon_id: "kernel-source",
    machine_id: "machine-source",
  }
  const options: Record<string, unknown> = {
    provider: "opencode",
    accountProfile: "default",
    model: "opencode/shared-model",
    effort: "",
    clientId: "composition-remote-pivot-control-test",
  }
  const composition = createCliWaitingRoomComposition({
    client,
    options,
    appLogger: { info: () => {}, warn: () => {} },
    formatError: (error: unknown) => error instanceof Error ? error.message : String(error),
    isAttached: () => false,
    kernelConnected: () => true,
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (next: typeof waitingRoomState) => {
      waitingRoomState = next
    },
    setWaitingRoomStateProjection: (next: typeof waitingRoomState) => {
      waitingRoomState = next
    },
    waitingRoomLaunchOwnershipRevision: () => 0,
    availableSessions: () => [],
    setAvailableSessions: () => {},
    waitingRoomProjects: () => [],
    setWaitingRoomProjects: () => {},
    providerCatalogState: () => providerCatalog,
    setProviderCatalogState: (next: ProviderCatalog) => {
      providerCatalog = next
    },
    providerCommandCatalogState: () => ({}),
    setProviderCommandCatalogState: () => {},
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    setWaitingRoomInventoryStatus: () => {},
    waitingRoomHiddenKernelController: {
      hideKernel: () => {},
      isKernelHidden: () => false,
    },
    relayStatusState: () => relay,
    setRelayStatusState: () => {},
    remoteMachinesState: () => [
      { machine_id: "machine-old", kernel_count: 1, available_providers: ["opencode"] },
      { machine_id: "machine-new", kernel_count: 1, available_providers: ["opencode"] },
    ],
    setRemoteMachinesState: () => {},
    remoteKernelsState: () => [
      { kernel_id: "kernel-old", machine_id: "machine-old", available_providers: ["opencode"] },
      { kernel_id: "kernel-new", machine_id: "machine-new", available_providers: ["opencode"] },
    ],
    setRemoteKernelsState: () => {},
    providerAccountsState: () => providerAccounts,
    setProviderAccountsState: () => {},
    terminalsState: () => [],
    setTerminalsState: () => {},
    slicesState: () => [],
    setSlicesState: () => {},
    externalProviderSessionsState: () => [],
    setExternalProviderSessionsState: () => {},
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    setExternalProviderSessionsPageState: () => {},
    pendingWorkspaceTarget: () => "/workspace",
    setPendingWorkspaceTarget: () => {},
    pendingWorktreeTarget: () => "/workspace",
    setPendingWorktreeTarget: () => {},
    preferencesState: () => ({}),
    setPreferencesState: () => {},
    setThemeRevision: () => {},
    resetTranscriptSyntax: () => {},
    applyResponseLayout: () => {},
    renderCommandCenter: () => {},
    rebuildTranscript: () => {},
    updateSessionChrome: () => {},
    syncCommandCenter: () => {},
    handleCloudCommand: async () => {},
    setPromptText: () => {},
    focusPrompt: () => {},
    openTerminalPairingDialog: async () => {},
    openSessionBrowserDialog: () => {},
    closeSessionBrowserDialog: () => {},
    attachBinding: async (session: { id: string }) => {
      resolveAttachBindingStarted?.()
      if (fixtureOptions.deferAttachBinding) {
        await attachBindingGate
      }
      attachedSessionIds.push(session.id)
    },
    rollbackAttachedSession: async (sessionId: string) => {
      rolledBackAttachmentIds.push(sessionId)
    },
    flashFooter: () => {},
    setKernelConnected: () => {},
    setDaemonDisconnected: () => {},
    sessionBrowserOpen: () => false,
    focusedProviderRun: () => null,
    focusedAgent: () => null,
    focusedAgentId: () => null,
    providerRunState: () => null,
    sessionState: () => ({ id: "session-1" }),
    applySessionState: () => {},
    setProviderRunState: () => {},
    appendNotice: () => {},
  } as never)

  return {
    composition,
    client,
    sourceClient,
    waitingRoomState: () => waitingRoomState,
    createdSessionOwners,
    deletedSessionIds,
    attachedSessionIds,
    rolledBackAttachmentIds,
    targetCatalogStarted,
    createSessionStarted,
    attachBindingStarted,
    releaseTargetCatalog: () => releaseTargetCatalog?.(),
    releaseCreateSession: () => releaseCreateSession?.(),
    releaseAttachBinding: () => releaseAttachBinding?.(),
    sourceCloseCount: () => sourceCloseCount,
    targetCloseCount: () => targetCloseCount,
    async cleanup() {
      releaseTargetCatalog?.()
      ;(LocalIpcClient.prototype as any).send = originalSend
      ;(LocalIpcClient.prototype as any).close = originalClose
      __setWaitingRoomWorktreeInventoryForTest(null)
    },
  }
}

function hasRequest(request: unknown, key: string): boolean {
  return typeof request === "object" && request !== null && key in request
}

function requestKind(request: unknown): string {
  return typeof request === "object" && request !== null ? Object.keys(request)[0] ?? "unknown" : typeof request
}

function publicSnapshot(kernelId: string, machineId: string) {
  return {
    schema_version: 1,
    generated_at_ms: 1,
    inventory_version: "1",
    structural_version: "1",
    activity_revision: "1",
    sessions: [],
    projects: [],
    relay_status: {
      configured: true,
      connected: true,
      daemon_id: kernelId,
      machine_id: machineId,
    },
  }
}

function createdSessionResponse() {
  return {
    SessionCreated: {
      session: {
        id: "created-session",
        project_id: "project-default",
        alias: null,
        workspace_id: "/workspace",
        worktree_id: "/workspace",
        created_at_ms: 1,
        status: "Active",
        agent_defaults: {
          provider: "opencode",
          model: "opencode/shared-model",
          effort: "",
          account_profile: "default",
          execution_mode: "build",
          permission_level: "yolo",
        },
        active_provider_run_id: null,
        attachment_ids: [],
        active_prompt: null,
        queued_prompts: [],
        focused_agent_id: null,
        max_agents: 1,
        agents: [],
        workflows: [],
        workflow_runs: [],
        workflow_watchdogs: [],
        workflow_consoles: [],
        config_state: {
          version: 1,
          values: {},
          updated_by_attachment_id: null,
        },
      },
    },
  }
}

function deletedSessionResponse() {
  return {
    SessionDeleted: {
      session: createdSessionResponse().SessionCreated.session,
    },
  }
}

function catalog(modelName: string): ProviderCatalog {
  return {
    all: [{
      id: "opencode",
      name: "OpenCode Zen",
      models: {
        "shared-model": {
          id: "shared-model",
          name: modelName,
          status: "active",
          variants: {},
        },
      },
    }],
    default: { opencode: "shared-model" },
    connected: ["opencode"],
  }
}

function account(profileId: string, label: string, isDefault: boolean) {
  return {
    owner_user_id: "test-user",
    provider: "opencode",
    profile_id: profileId,
    label,
    origin: isDefault ? "default" : "chariox_created",
    is_default: isDefault,
    auth_state: "authenticated",
    usage: {
      profile_id: profileId,
      provider: "opencode",
      availability: "available",
      source: "test",
    },
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import { buildModelItems } from "./command-center-dynamic-items.js"
import type { LocalIpcClient } from "./ipc.js"
import { getProviderCatalog } from "./provider-api.js"
import { catalogModelOptions, selectConfiguredModel, type ProviderCatalog } from "./provider-catalog.js"
import { deriveWaitingRoomActivationDecision } from "./waiting-room-controller.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
import {
  loadProviderCatalogForKernel,
  providerCatalogExecutionLocation,
} from "./waiting-room-provider-catalog.js"
import { createWaitingRoomReconcileController } from "./waiting-room-reconcile-controller.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

test("target-kernel catalog hydration keeps discovered model efforts and qualified routing", async () => {
  const requests: unknown[] = []
  const catalog = await loadProviderCatalogForKernel(
    clientReturning({
      ProviderCatalog: {
        catalog: {
          all: [
            {
              id: "opencode",
              name: "OpenCode Zen",
              models: {
                "authoritative-model": {
                  id: "authoritative-model",
                  name: "Authoritative model",
                  status: "active",
                  variants: { medium: {}, high: {} },
                },
              },
            },
            {
              id: "opencode-go",
              name: "OpenCode Go",
              models: {
                "authoritative-model": {
                  id: "authoritative-model",
                  name: "Authoritative model",
                  status: "active",
                  variants: { low: {}, max: {} },
                },
              },
            },
          ],
          default: {
            opencode: "authoritative-model",
            "opencode-go": "authoritative-model",
          },
          connected: ["opencode", "opencode-go"],
        } satisfies ProviderCatalog,
      },
    }, requests),
    undefined,
    { providerId: "opencode", accountProfileId: "managed-account" },
  )

  assert.deepEqual(requests, [{
    GetProviderCatalog: {
      provider: "opencode",
      account_profiles: { opencode: "managed-account" },
      execution_location: { kind: "local" },
    },
  }])
  assert.equal(catalog.source, "daemon")

  const options = catalogModelOptions(catalog, "opencode")
  assert.deepEqual(options.map((option) => option.id).sort(), [
    "opencode-go/authoritative-model",
    "opencode/authoritative-model",
  ])
  assert.deepEqual(
    options.find((option) => option.id === "opencode/authoritative-model")?.variants,
    ["medium", "high"],
  )
  assert.deepEqual(
    options.find((option) => option.id === "opencode-go/authoritative-model")?.variants,
    ["low", "max"],
  )
  assert.equal(
    selectConfiguredModel(catalog, "opencode-go/authoritative-model", "opencode")?.providerId,
    "opencode-go",
  )
})

test("target-kernel catalog hydration surfaces discovery failure without fallback models", async () => {
  await assert.rejects(
    loadProviderCatalogForKernel(clientRejecting(new Error("target catalog unavailable")), undefined, {
      providerId: "codex",
      accountProfileId: "managed-account",
    }),
    /target catalog unavailable/,
  )
})

test("pivot then self-target/account change, other-kernel selection, and slice resolve catalog location", async () => {
  const requests: unknown[] = []
  const pendingRefreshes: Promise<ProviderCatalog>[] = []
  const targetClient = clientRejectingSelfWorker(requests, "kernel-a")
  let activeDirectTargetKernelId: string | null = null
  let currentState = createWaitingRoomState(
    [],
    recordedOpenCodeNativeCatalog(),
    "opencode",
    "opencode/big-pickle",
    "",
  )
  const controller = createWaitingRoomReconcileController({
    getCurrentState: () => currentState,
    setWaitingRoomState: (nextState) => {
      currentState = nextState
    },
    getSessions: () => [] as SessionListEntry[],
    getProviderCatalog: () => recordedOpenCodeNativeCatalog(),
    getRemoteState: () => ({}) as WaitingRoomRemoteState,
    getThemeRegistry: () => ({}) as ThemeRegistry,
    getCurrentProvider: () => currentState.providerId,
    getCurrentModel: () => currentState.modelId,
    setProviderDefaults: () => {},
    applyTheme: (themeId) => themeId,
    resetTranscriptSyntax: () => {},
    bumpThemeRevision: () => {},
    saveUiThemePreference: () => {},
    mergeUiThemePreference: () => {},
    applyResponseLayout: () => {},
    renderCommandCenter: () => {},
    saveProviderPreferences: () => {},
    isAttached: () => false,
    rebuildTranscript: () => {},
    updateSessionChrome: () => {},
    syncCommandCenter: () => {},
    refreshProviderCatalogForSelection: (state) => {
      pendingRefreshes.push(getProviderCatalog(targetClient, undefined, {
        provider: state.providerId,
        accountProfile: state.accountProfileId ?? "default",
        executionLocation: providerCatalogExecutionLocation(state, activeDirectTargetKernelId),
      }, false))
    },
    deriveStateUpdate: ({ nextState }) => ({
      normalizedState: nextState,
      nextProvider: nextState.providerId,
      nextModel: nextState.modelId,
      nextEffort: nextState.effort,
      shouldPersistProviderPreferences: false,
    }),
  })

  // replaceClientForKernel records this direct target when the pivot commits.
  activeDirectTargetKernelId = "kernel-a"
  controller.reconcile({
    ...currentState,
    selectedKernelRef: "kernel-a",
  })
  controller.reconcile({
    ...currentState,
    providerId: "codex",
    accountProfileId: "imported-codex",
    modelId: "gpt-5.6-luna",
    effort: "high",
  })
  controller.reconcile({
    ...currentState,
    selectedKernelRef: "kernel-b",
  })
  controller.reconcile({
    ...currentState,
    sliceSelectionId: "slice-1",
  })

  await Promise.all(pendingRefreshes)
  assert.deepEqual(
    requests.map((request) => (request as {
      GetProviderCatalog: { provider: string; account_profiles: Record<string, string>; execution_location: unknown }
    }).GetProviderCatalog),
    [
      {
        provider: "opencode",
        account_profiles: { opencode: "default" },
        execution_location: { kind: "local" },
      },
      {
        provider: "codex",
        account_profiles: { codex: "imported-codex" },
        execution_location: { kind: "local" },
      },
      {
        provider: "codex",
        account_profiles: { codex: "imported-codex" },
        execution_location: { kind: "worker", kernel_ref: "kernel-b" },
      },
      {
        provider: "codex",
        account_profiles: { codex: "imported-codex" },
        execution_location: { kind: "slice", slice_ref: "slice-1" },
      },
    ],
  )
})

test("recorded OpenCode native discovery reaches selectors without inventing Go entitlement", async () => {
  const catalog = await loadProviderCatalogForKernel(
    clientReturning({
      ProviderCatalog: { catalog: recordedOpenCodeNativeCatalog() },
    }, []),
    undefined,
    { providerId: "opencode", accountProfileId: "default" },
  )

  assert.equal(catalog.source, "daemon")
  assert.deepEqual(catalog.connected, ["opencode"])

  const entitledZenOptions = catalogModelOptions(catalog, "opencode")
  assert.deepEqual(entitledZenOptions.map((option) => option.id), ["opencode/big-pickle"])
  assert.equal(entitledZenOptions.some((option) => option.providerId === "opencode-go"), false)

  const state = createWaitingRoomState([], catalog, "opencode", "opencode/big-pickle", "")
  assert.equal(
    waitingRoomRows(state, [], catalog).find((row) => row.id === "model")?.value,
    "OpenCode Zen Big Pickle",
  )
  assert.deepEqual(
    buildModelItems("/model big", {
      providerCatalog: catalog,
      currentProvider: "opencode",
      currentModel: "opencode/big-pickle",
      currentVariant: "",
    }).map((item) => ({ label: item.label, value: item.value })),
    [{ label: "OpenCode Zen Big Pickle", value: "opencode/big-pickle" }],
  )

  // The installed server reports Go in `all` but not `connected` without a
  // Go entitlement. A separate connected fixture only checks rendering and
  // routing for that entitlement; it does not make the live account claim it.
  const entitledBoth = { ...catalog, connected: ["opencode", "opencode-go"] }
  assert.deepEqual(
    catalogModelOptions(entitledBoth, "opencode")
      .map((option) => ({ id: option.id, providerId: option.providerId, providerName: option.providerName }))
      .sort((left, right) => left.id.localeCompare(right.id)),
    [
      { id: "opencode-go/gpt-5.6-luna", providerId: "opencode-go", providerName: "OpenCode Go" },
      { id: "opencode/big-pickle", providerId: "opencode", providerName: "OpenCode Zen" },
    ],
  )
  const goOption = catalogModelOptions(entitledBoth, "opencode")
    .find((option) => option.id === "opencode-go/gpt-5.6-luna")
  assert.deepEqual(
    goOption && { id: goOption.id, providerName: goOption.providerName, variants: goOption.variants },
    {
      id: "opencode-go/gpt-5.6-luna",
      providerName: "OpenCode Go",
      variants: ["none", "low", "medium", "high", "xhigh", "max"],
    },
  )
  assert.deepEqual(
    buildModelItems("/model luna", {
      providerCatalog: entitledBoth,
      currentProvider: "opencode",
      currentModel: "opencode-go/gpt-5.6-luna",
      currentVariant: "max",
    }).map((item) => ({ label: item.label, value: item.value })),
    [{ label: "OpenCode Go GPT-5.6 Luna", value: "opencode-go/gpt-5.6-luna" }],
  )

  const entitledGoOnly = { ...catalog, connected: ["opencode-go"] }
  const goOnlyOptions = catalogModelOptions(entitledGoOnly, "opencode")
  assert.deepEqual(
    goOnlyOptions.map((option) => ({
      id: option.id,
      providerId: option.providerId,
      providerName: option.providerName,
    })),
    [{ id: "opencode-go/gpt-5.6-luna", providerId: "opencode-go", providerName: "OpenCode Go" }],
  )
  assert.equal(
    selectConfiguredModel(entitledGoOnly, "opencode-go/gpt-5.6-luna", "opencode")?.providerId,
    "opencode-go",
  )
  assert.deepEqual(
    buildModelItems("/model luna", {
      providerCatalog: entitledGoOnly,
      currentProvider: "opencode",
      currentModel: "opencode-go/gpt-5.6-luna",
      currentVariant: "max",
    }).map((item) => ({ label: item.label, value: item.value })),
    [{ label: "OpenCode Go GPT-5.6 Luna", value: "opencode-go/gpt-5.6-luna" }],
  )
  const goOnlyState = createWaitingRoomState(
    [],
    entitledGoOnly,
    "opencode",
    "opencode-go/gpt-5.6-luna",
    "max",
  )
  assert.equal(
    waitingRoomRows(goOnlyState, [], entitledGoOnly).find((row) => row.id === "model")?.value,
    "OpenCode Go GPT-5.6 Luna",
  )
})

test("selected OpenCode native provider/model routes unchanged on local and remote launches", () => {
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
    const zenCatalog = recordedOpenCodeNativeCatalog()
    const localState = createWaitingRoomState(
      [],
      zenCatalog,
      "opencode",
      "opencode/big-pickle",
      "",
    )
    const localDecision = deriveWaitingRoomActivationDecision({
      state: localState,
      sessions: [],
      catalog: zenCatalog,
      currentProvider: "opencode",
      currentModel: "opencode/big-pickle",
    })
    assert.equal(localDecision.action, "create")
    if (localDecision.action !== "create") return
    assert.deepEqual({
      provider: localDecision.launch.provider,
      model: localDecision.launch.model,
      ownerMachineRef: localDecision.launch.ownerMachineRef,
      ownerKernelRef: localDecision.launch.ownerKernelRef,
    }, {
      provider: "opencode",
      model: "opencode/big-pickle",
      ownerMachineRef: "local",
      ownerKernelRef: "local",
    })

    const goCatalog = { ...zenCatalog, connected: ["opencode", "opencode-go"] }
    const remoteState = {
      ...createWaitingRoomState(
        [],
        goCatalog,
        "opencode",
        "opencode-go/gpt-5.6-luna",
        "max",
      ),
      selectedMachineRef: "machine-b",
      selectedKernelRef: "kernel-b",
    }
    const remoteDecision = deriveWaitingRoomActivationDecision({
      state: remoteState,
      sessions: [],
      catalog: goCatalog,
      currentProvider: "opencode",
      currentModel: "opencode-go/gpt-5.6-luna",
      remote: {
        machines: [{ machine_id: "machine-b", kernel_count: 1 }],
        kernels: [{ kernel_id: "kernel-b", machine_id: "machine-b", available_providers: ["opencode"] }],
      },
    })
    assert.equal(remoteDecision.action, "create")
    if (remoteDecision.action !== "create") return
    assert.deepEqual({
      provider: remoteDecision.launch.provider,
      model: remoteDecision.launch.model,
      ownerMachineRef: remoteDecision.launch.ownerMachineRef,
      ownerKernelRef: remoteDecision.launch.ownerKernelRef,
    }, {
      provider: "opencode",
      model: "opencode-go/gpt-5.6-luna",
      ownerMachineRef: "machine-b",
      ownerKernelRef: "kernel-b",
    })
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

function recordedOpenCodeNativeCatalog(): ProviderCatalog {
  // These native ids, names, defaults, and variants were recorded from the
  // installed OpenCode 1.18.23 `/provider` response. Only the two native
  // plan entries needed by this selector regression are retained here.
  return {
    all: [
      {
        id: "opencode",
        name: "OpenCode Zen",
        models: {
          "big-pickle": {
            id: "big-pickle",
            name: "Big Pickle",
            status: "active",
            variants: {},
          },
        },
      },
      {
        id: "opencode-go",
        name: "OpenCode Go",
        models: {
          "gpt-5.6-luna": {
            id: "gpt-5.6-luna",
            name: "GPT-5.6 Luna",
            status: "active",
            variants: {
              none: {},
              low: {},
              medium: {},
              high: {},
              xhigh: {},
              max: {},
            },
          },
        },
      },
    ],
    default: {
      opencode: "big-pickle",
      "opencode-go": "gpt-5.6-luna",
    },
    connected: ["opencode"],
  }
}

function clientReturning(response: Record<string, unknown>, requests: unknown[]): LocalIpcClient {
  return {
    send: async (request: unknown) => {
      requests.push(request)
      return response
    },
  } as unknown as LocalIpcClient
}

function clientRejecting(error: Error): LocalIpcClient {
  return {
    send: async () => {
      throw error
    },
  } as unknown as LocalIpcClient
}

function clientRejectingSelfWorker(requests: unknown[], directKernelRef: string): LocalIpcClient {
  return {
    send: async (request: unknown) => {
      requests.push(request)
      const location = (request as {
        GetProviderCatalog?: { execution_location?: { kind?: string; kernel_ref?: string } }
      }).GetProviderCatalog?.execution_location
      if (location?.kind === "worker" && location.kernel_ref === directKernelRef) {
        throw new Error("target rejects worker:self for a directly active kernel")
      }
      return {
        ProviderCatalog: { catalog: recordedOpenCodeNativeCatalog() },
      }
    },
  } as unknown as LocalIpcClient
}

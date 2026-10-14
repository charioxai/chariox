import assert from "node:assert/strict"
import test from "node:test"
import type { ProviderAccountProfile } from "@chariox/kernel-client"

import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import {
  createWaitingRoomReconcileController,
  type WaitingRoomProviderDefaults,
} from "./waiting-room-reconcile-controller.js"
import type { WaitingRoomStateUpdate } from "./waiting-room-controller.js"
import { selectedProviderAccount } from "./waiting-room-provider-accounts.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

test("waiting room reconcile controller applies normalized state and refreshes detached UI", () => {
  const nextState = waitingRoomState({ themeId: "dark" })
  const harness = createHarness({
    currentState: waitingRoomState({ themeId: "dark" }),
    update: {
      normalizedState: nextState,
      nextProvider: "opencode",
      nextModel: "gpt-5.4",
      nextEffort: "medium",
      shouldPersistProviderPreferences: false,
    },
    attached: false,
  })

  const reconciled = harness.controller.reconcile(waitingRoomState({ themeId: "dark" }))

  assert.equal(reconciled, nextState)
  assert.equal(harness.state.currentState, nextState)
  assert.deepEqual(harness.state.defaults, {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "medium",
  })
  assert.deepEqual(harness.calls, [
    "setWaitingRoomState",
    "setProviderDefaults",
    "rebuildTranscript",
    "updateSessionChrome",
    "syncCommandCenter",
  ])
})

test("waiting room reconcile controller applies derived inventory through the projection setter", () => {
  const nextState = waitingRoomState({
    modelId: "gpt-5.6-sol",
    projectSelectionId: "default",
  })
  const harness = createHarness({
    currentState: waitingRoomState({
      modelId: "gpt-5.6-luna",
      projectSelectionId: "existing:project-one",
    }),
    update: {
      normalizedState: nextState,
      nextProvider: "codex",
      nextModel: "gpt-5.6-sol",
      nextEffort: "low",
      shouldPersistProviderPreferences: false,
    },
    attached: true,
  })

  harness.controller.reconcileProjection(harness.state.currentState)

  assert.equal(harness.state.currentState, nextState)
  assert.deepEqual(harness.calls, [
    "setProjectedWaitingRoomState",
    "setProviderDefaults",
    "updateSessionChrome",
    "syncCommandCenter",
  ])
})

test("waiting room reconcile controller applies and persists theme changes", () => {
  const harness = createHarness({
    currentState: waitingRoomState({ themeId: "dark" }),
    update: {
      normalizedState: waitingRoomState({ themeId: "light" }),
      nextProvider: "opencode",
      nextModel: "gpt-5.4",
      nextEffort: "medium",
      shouldPersistProviderPreferences: false,
    },
    appliedThemeId: "light-resolved",
    attached: true,
  })

  harness.controller.reconcile(waitingRoomState({ themeId: "light" }))

  assert.deepEqual(harness.state.savedUiThemes, ["light-resolved"])
  assert.deepEqual(harness.state.mergedUiThemes, ["light-resolved"])
  assert.deepEqual(harness.calls, [
    "setWaitingRoomState",
    "setProviderDefaults",
    "applyTheme",
    "resetTranscriptSyntax",
    "bumpThemeRevision",
    "saveUiThemePreference",
    "mergeUiThemePreference",
    "applyResponseLayout",
    "renderCommandCenter",
    "updateSessionChrome",
    "syncCommandCenter",
  ])
})

test("waiting room reconcile controller persists provider defaults when policy requests it", () => {
  const harness = createHarness({
    currentState: waitingRoomState({ themeId: "dark" }),
    update: {
      normalizedState: waitingRoomState({ themeId: "dark" }),
      nextProvider: "codex",
      nextModel: "gpt-5.5",
      nextEffort: "high",
      shouldPersistProviderPreferences: true,
    },
    attached: true,
  })

  harness.controller.reconcile(waitingRoomState({ themeId: "dark" }))

  assert.deepEqual(harness.state.savedProviderPreferences, [{
    provider: "codex",
    preferences: {
      model: "gpt-5.5",
      effort: "high",
    },
  }])
})

for (const explicit of [false, true]) {
  test(`MP-08/MP-10/MP-11 catalog follows default promotion with ${explicit ? "an explicit account" : "follow-default intent"}`, () => {
    const catalog = (model: string, effort: string): ProviderCatalog => ({
      all: [{ id: "codex", name: "Codex", models: {
        [model]: { id: model, name: model, status: "active", variants: { [effort]: {} } },
      } }],
      default: { codex: model },
      connected: ["codex"],
    })
    const account = (id: string, isDefault: boolean): ProviderAccountProfile => ({
      owner_user_id: "local", provider: "codex", profile_id: id, label: id,
      origin: "linked", is_default: isDefault, auth_state: "authenticated",
      usage: { profile_id: id, provider: "codex", availability: "unavailable", source: "test" },
    })
    const requests: string[] = []
    const harness = createHarness({
      currentState: waitingRoomState({
        providerId: "codex", accountProfileId: explicit ? "A" : "default",
        modelId: "codex/model-a", effort: "low", sliceSelectionId: "none",
        selectedMachineRef: "local", selectedKernelRef: "local",
      }),
      catalog: catalog("model-a", "low"),
      remote: { providerAccounts: [account("A", true)] },
      attached: false,
      onCatalogRefresh: (next) => {
        requests.push(selectedProviderAccount(harness.state.remote.providerAccounts,
          next.providerId, next.accountProfileId)!.profile_id)
      },
    })
    harness.controller.reconcileProjection(harness.state.currentState)
    assert.deepEqual(requests, [])

    harness.state.remote = { providerAccounts: [account("A", false), account("B", true)] }
    harness.controller.reconcileProjection(harness.state.currentState)
    assert.equal(harness.state.currentState.accountProfileId, explicit ? "A" : "default")
    assert.deepEqual(requests, explicit ? [] : ["B"])
    if (!explicit) {
      // The production catalog callback delivers B asynchronously, then reconciles again.
      harness.state.catalog = catalog("model-b", "high")
      harness.controller.reconcileProjection(harness.state.currentState)
    }
    assert.equal(harness.state.currentState.modelId, explicit ? "codex/model-a" : "codex/model-b")
    assert.equal(harness.state.currentState.effort, explicit ? "low" : "high")
    assert.equal(harness.state.currentState.accountProfileId, explicit ? "A" : "default")
    harness.controller.reconcile(harness.state.currentState)
    assert.deepEqual(requests, explicit ? [] : ["B"], "catalog delivery and UI updates must not loop")
  })
}

function createHarness(options: {
  currentState: WaitingRoomState
  update?: WaitingRoomStateUpdate
  attached: boolean
  appliedThemeId?: string
  catalog?: ProviderCatalog
  remote?: WaitingRoomRemoteState
  onCatalogRefresh?: (state: WaitingRoomState) => void
}) {
  const calls: string[] = []
  const state = {
    currentState: options.currentState,
    catalog: options.catalog ?? ({} as ProviderCatalog),
    remote: options.remote ?? ({} as WaitingRoomRemoteState),
    defaults: null as WaitingRoomProviderDefaults | null,
    savedUiThemes: [] as string[],
    mergedUiThemes: [] as string[],
    savedProviderPreferences: [] as Array<{
      provider: BackendProviderId
      preferences: { model: string; effort: string }
    }>,
  }
  const controller = createWaitingRoomReconcileController({
    getCurrentState: () => state.currentState,
    setWaitingRoomState: (nextState) => {
      calls.push("setWaitingRoomState")
      state.currentState = nextState
    },
    setProjectedWaitingRoomState: (nextState) => {
      calls.push("setProjectedWaitingRoomState")
      state.currentState = nextState
    },
    getSessions: () => [] as SessionListEntry[],
    getProviderCatalog: () => state.catalog,
    getRemoteState: () => state.remote,
    getThemeRegistry: () => DEFAULT_THEME_REGISTRY,
    getCurrentProvider: () => "opencode",
    getCurrentModel: () => "gpt-5.4",
    setProviderDefaults: (defaults) => {
      calls.push("setProviderDefaults")
      state.defaults = defaults
    },
    applyTheme: () => {
      calls.push("applyTheme")
      return options.appliedThemeId ?? state.currentState.themeId
    },
    resetTranscriptSyntax: () => {
      calls.push("resetTranscriptSyntax")
    },
    bumpThemeRevision: () => {
      calls.push("bumpThemeRevision")
    },
    saveUiThemePreference: (themeId) => {
      calls.push("saveUiThemePreference")
      state.savedUiThemes.push(themeId)
    },
    mergeUiThemePreference: (themeId) => {
      calls.push("mergeUiThemePreference")
      state.mergedUiThemes.push(themeId)
    },
    applyResponseLayout: () => {
      calls.push("applyResponseLayout")
    },
    renderCommandCenter: () => {
      calls.push("renderCommandCenter")
    },
    saveProviderPreferences: (provider, preferences) => {
      calls.push("saveProviderPreferences")
      state.savedProviderPreferences.push({ provider, preferences })
    },
    isAttached: () => options.attached,
    rebuildTranscript: () => {
      calls.push("rebuildTranscript")
    },
    updateSessionChrome: () => {
      calls.push("updateSessionChrome")
    },
    syncCommandCenter: () => {
      calls.push("syncCommandCenter")
    },
    ...(options.onCatalogRefresh ? { refreshProviderCatalogForSelection: options.onCatalogRefresh } : {}),
    ...(options.update ? { deriveStateUpdate: () => options.update! } : {}),
  })
  return { calls, state, controller }
}

function waitingRoomState(overrides: Partial<WaitingRoomState>): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    providerId: "opencode",
    modelId: "gpt-5.4",
    effort: "medium",
    themeId: "dark",
    workspaceSelectionId: "current",
    worktreeSelectionId: "current",
    sliceSelectionId: "local",
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  } as WaitingRoomState
}

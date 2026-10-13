import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { cycleWaitingRoomFocusedValue } from "./waiting-room-value-cycling.js"
import {
  createWaitingRoomState,
  normalizeWaitingRoomState,
} from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("waiting room focused value cycling normalizes provider and model changes", () => {
  const catalog = fallbackProviderCatalog()
  const normalizeState = (state: WaitingRoomState) => normalizeWaitingRoomState(
    state,
    [],
    catalog,
    DEFAULT_THEME_REGISTRY,
  )
  let state: WaitingRoomState = {
    ...createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high"),
    focus: "provider" as const,
  }

  state = cycleWaitingRoomFocusedValue(state, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState,
  })
  assert.equal(state.providerId, "codex")

  state = {
    ...state,
    focus: "model",
  }
  const next = cycleWaitingRoomFocusedValue(state, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState,
  })
  assert.equal(next.providerId, state.providerId)
  assert.notEqual(next.modelId, "")
})

test("waiting room focused value cycling updates local value selectors", () => {
  const catalog = fallbackProviderCatalog()
  const state = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high")

  const effortState = cycleWaitingRoomFocusedValue({ ...state, focus: "effort" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState: (next) => next,
  })
  assert.notEqual(effortState.effort, "")

  const themeState = cycleWaitingRoomFocusedValue({ ...state, focus: "theme", themeId: "sober" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState: (next) => next,
  })
  assert.equal(themeState.themeId, "matrix")

  const sliceState = cycleWaitingRoomFocusedValue({ ...state, focus: "slice" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    remote: { slices: [slice()] },
    normalizeState: (next) => next,
  })
  assert.equal(sliceState.sliceSelectionId, "new")
  assert.equal(sliceState.sliceDisplayMode, "headless")

  const liveSyncState = cycleWaitingRoomFocusedValue({ ...state, focus: "live-sync" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState: (next) => next,
  })
  assert.equal(liveSyncState.workspaceLiveSyncMode, "managed")

  const archivedProjectsState = cycleWaitingRoomFocusedValue({
    ...state,
    focus: "archived-projects",
    showArchivedProjects: false,
  }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    normalizeState: (next) => next,
  })
  assert.equal(archivedProjectsState.showArchivedProjects, true)

  const sliceDisplayState = cycleWaitingRoomFocusedValue({ ...sliceState, focus: "slice" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    remote: { slices: [slice()] },
    normalizeState: (next) => next,
  })
  assert.equal(sliceDisplayState.sliceDisplayMode, "headed")
})

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "linux-dev",
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "running",
    workspace_mount: null,
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    providers: ["codex"],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

test("MP-08 waiting room provider cycling offers only kernel-installed harnesses", () => {
  const catalog = fallbackProviderCatalog()
  const remote = { installedProviders: ["claude", "codex"] }
  const normalizeState = (state: WaitingRoomState) => normalizeWaitingRoomState(
    state,
    [],
    catalog,
    DEFAULT_THEME_REGISTRY,
    remote,
  )
  const initial = normalizeState({
    ...createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high"),
    focus: "provider" as const,
  })
  assert.equal(initial.providerId, "codex", "an uninstalled selected provider falls back to an installed one")

  const seen = new Set<string>()
  let state = initial
  for (let step = 0; step < 6; step += 1) {
    state = cycleWaitingRoomFocusedValue(state, 1, {
      catalog,
      themeRegistry: DEFAULT_THEME_REGISTRY,
      remote,
      normalizeState,
    })
    seen.add(state.providerId)
  }
  assert.deepEqual([...seen].sort(), ["claude-headless", "claude-p", "codex"])

  const olderKernel = cycleWaitingRoomFocusedValue({ ...initial, providerId: "claude-p" }, 1, {
    catalog,
    themeRegistry: DEFAULT_THEME_REGISTRY,
    remote: {},
    normalizeState: (next) => next,
  })
  assert.equal(olderKernel.providerId, "opencode", "a kernel that reports nothing keeps every provider")
})

test("MP-08 Account row cycling ends with an add-new-account value that never becomes the launch account", () => {
  const catalog = fallbackProviderCatalog()
  const account = (profileId: string) => ({
    owner_user_id: "local", provider: "codex", profile_id: profileId, label: profileId, origin: "default" as const,
    is_default: profileId === "codex-1", auth_state: "authenticated" as const,
    usage: { profile_id: profileId, provider: "codex", availability: "unavailable" as const, source: "test" },
  })
  const remote = { providerAccounts: [account("codex-1"), account("codex-2")], installedProviders: ["codex"] }
  const normalizeState = (state: WaitingRoomState) => normalizeWaitingRoomState(state, [], catalog, DEFAULT_THEME_REGISTRY, remote)
  let state = normalizeState({ ...createWaitingRoomState([], catalog, "codex", "codex/gpt-5.4", "high"), focus: "account" as const, accountProfileId: "codex-2" })
  const cycle = (delta: number) => { state = cycleWaitingRoomFocusedValue(state, delta, { catalog, themeRegistry: DEFAULT_THEME_REGISTRY, remote, normalizeState }) }
  cycle(1)
  assert.equal(state.accountAddSelected, true)
  assert.equal(state.accountProfileId, "codex-2")
  cycle(1)
  assert.equal(state.accountAddSelected, false)
  assert.equal(state.accountProfileId, "codex-1")
  cycle(-1)
  assert.equal(state.accountAddSelected, true)
  state = normalizeState({ ...state, focus: "model" })
  assert.equal(state.accountAddSelected, undefined, "leaving the row drops the add value")
})

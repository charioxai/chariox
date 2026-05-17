import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { cycleWaitingRoomFocusedValue } from "./waiting-room-value-cycling.js"
import {
  createWaitingRoomState,
  normalizeWaitingRoomState,
  type WaitingRoomState,
} from "./waiting-room.js"

test("waiting room focused value cycling normalizes provider and model changes", () => {
  const catalog = fallbackProviderCatalog()
  const normalizeState = (state: WaitingRoomState) => normalizeWaitingRoomState(
    state,
    [],
    catalog,
    DEFAULT_THEME_REGISTRY,
  )
  let state: WaitingRoomState = {
    ...createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high"),
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
  const state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")

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
  assert.equal(sliceState.sliceSelectionId, "slice-1")
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

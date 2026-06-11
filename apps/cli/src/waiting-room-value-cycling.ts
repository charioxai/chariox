import {
  BACKEND_PROVIDER_IDS,
  catalogModelOptions,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  DEFAULT_THEME_REGISTRY,
  normalizeThemeName,
  themeOptions,
  type ThemeRegistry,
} from "./theme-registry.js"
import { waitingRoomEfforts, waitingRoomModel } from "./waiting-room-choice.js"
import {
  cycleWaitingRoomSliceSelection,
  waitingRoomSlices,
} from "./waiting-room-slices.js"
import {
  cycleWaitingRoomLaunchKernel,
  cycleWaitingRoomLaunchMachine,
} from "./waiting-room-runtime-placement.js"
import { normalizeWaitingRoomState } from "./waiting-room-state.js"
import { cycleWaitingRoomWorktreeSelectionId } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export function cycleWaitingRoomValue(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  delta: number,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
  remote: WaitingRoomRemoteState = {},
) {
  return cycleWaitingRoomFocusedValue(state, delta, {
    catalog,
    themeRegistry,
    remote,
    normalizeState: (next) => normalizeWaitingRoomState(next, sessions, catalog, themeRegistry, remote),
  })
}

export function cycleWaitingRoomFocusedValue(
  state: WaitingRoomState,
  delta: number,
  context: {
    catalog: ProviderCatalog
    themeRegistry: ThemeRegistry
    remote?: WaitingRoomRemoteState
    normalizeState: (state: WaitingRoomState) => WaitingRoomState
  },
) {
  const remote = context.remote ?? {}
  if (state.focus === "model") {
    const options = catalogModelOptions(context.catalog, state.providerId)
    if (options.length === 0) {
      return state
    }
    const index = Math.max(0, options.findIndex((option) => option.id === state.modelId))
    const next = options[modulo(index + delta, options.length)]!
    return context.normalizeState({
      ...state,
      modelId: next.id,
    })
  }
  if (state.focus === "provider") {
    const index = Math.max(0, BACKEND_PROVIDER_IDS.indexOf(state.providerId))
    return context.normalizeState({
      ...state,
      providerId: BACKEND_PROVIDER_IDS[modulo(index + delta, BACKEND_PROVIDER_IDS.length)]!,
    })
  }
  if (state.focus === "effort") {
    const efforts = waitingRoomEfforts(waitingRoomModel(state, context.catalog))
    const index = Math.max(0, efforts.indexOf(state.effort))
    return {
      ...state,
      effort: efforts[modulo(index + delta, efforts.length)] ?? "",
    }
  }
  if (state.focus === "theme") {
    const options = themeOptions(context.themeRegistry)
    const ids = options.map((option) => option.id)
    const index = Math.max(0, ids.indexOf(normalizeThemeName(state.themeId, context.themeRegistry)))
    return {
      ...state,
      themeId: ids[modulo(index + delta, ids.length)] ?? normalizeThemeName(state.themeId, context.themeRegistry),
    }
  }
  if (state.focus === "worktree") {
    return {
      ...state,
      worktreeSelectionId: cycleWaitingRoomWorktreeSelectionId(state.worktreeSelectionId, delta),
    }
  }
  if (state.focus === "launch-machine") {
    return context.normalizeState(cycleWaitingRoomLaunchMachine(state, remote, delta))
  }
  if (state.focus === "launch-kernel") {
    return context.normalizeState(cycleWaitingRoomLaunchKernel(state, remote, delta))
  }
  if (state.focus === "live-sync") {
    const modes: readonly WaitingRoomState["workspaceLiveSyncMode"][] = ["off", "managed", "tracked"]
    const index = Math.max(0, modes.indexOf(state.workspaceLiveSyncMode))
    return {
      ...state,
      workspaceLiveSyncMode: modes[modulo(index + delta, modes.length)] ?? "off",
    }
  }
  if (state.focus === "metaagent") {
    return {
      ...state,
      createMetaagent: !state.createMetaagent,
    }
  }
  if (state.focus === "slice") {
    return cycleWaitingRoomSliceSelection(
      state,
      waitingRoomSlices(remote, {
        worktreeSelectionId: state.worktreeSelectionId,
        selectedMachineRef: state.selectedMachineRef,
        selectedKernelRef: state.selectedKernelRef,
      }),
      delta,
    )
  }
  return state
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

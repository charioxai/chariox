import {
  catalogModelOptions,
  type CatalogModelOption,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachines,
} from "./waiting-room-remote-rows.js"
import { waitingRoomSessions } from "./waiting-room-session-rows.js"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import {
  selectedWaitingRoomSliceRef,
  selectedWaitingRoomSliceCreateMode,
  waitingRoomSelectedSlice,
  waitingRoomSlices,
} from "./waiting-room-slices.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export function waitingRoomModel(state: WaitingRoomState, catalog: ProviderCatalog) {
  return catalogModelOptions(catalog, state.providerId).find((option) => option.id === state.modelId) ?? null
}

export function waitingRoomEfforts(option: CatalogModelOption | null) {
  if (!option || option.variants.length === 0) {
    return [""]
  }
  return option.variants
}

export function waitingRoomChoice(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  remote: WaitingRoomRemoteState = {},
) {
  const visibleSessions = waitingRoomSessions(sessions)
  const model = waitingRoomModel(state, catalog)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const terminals = waitingRoomTerminals(remote)
  const slices = waitingRoomSlices(remote, { worktreeSelectionId: state.worktreeSelectionId })
  const allSlices = waitingRoomAllSlices(remote)
  return {
    session: visibleSessions[state.sessionIndex] ?? null,
    remoteMachine: remoteMachines[state.machineIndex] ?? null,
    remoteKernel: remoteKernels[state.remoteKernelIndex] ?? null,
    terminal: terminals[state.terminalIndex] ?? null,
    sliceInventory: allSlices[state.sliceIndex ?? 0] ?? null,
    slice: waitingRoomSelectedSlice(state.sliceSelectionId, slices),
    sliceRef: selectedWaitingRoomSliceRef(state.sliceSelectionId, slices),
    sliceCreate: selectedWaitingRoomSliceCreateMode(state.sliceSelectionId, state.sliceDisplayMode),
    providerId: state.providerId,
    model,
    effort: state.effort,
  }
}

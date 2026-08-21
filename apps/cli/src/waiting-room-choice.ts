import {
  externalProviderSessionAtSelection,
  externalProviderSessionPageSessions,
} from "@chariox/kernel-client/external-provider-sessions"
import {
  catalogModelOptions,
  providerCatalogIsLocalFallback,
  type CatalogModelOption,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachines,
} from "./waiting-room-remote-rows.js"
import { waitingRoomLaunchPlacement } from "@chariox/kernel-client/waiting-room-runtime-placement"
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
import { normalizeWaitingRoomProjectSelectionId, projectSelectionFromId } from "./waiting-room-projects.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import { selectedProviderAccount } from "./waiting-room-provider-accounts.js"

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
  const externalProviderSessions = externalProviderSessionPageSessions(remote)
  const placement = waitingRoomLaunchPlacement(state, remote)
  const slices = waitingRoomSlices(remote, {
    worktreeSelectionId: state.worktreeSelectionId,
    projectSelectionId: state.projectSelectionId,
    repositoryMode: state.managedRepositoryMode,
    selectedMachineRef: placement.machineRef,
    selectedKernelRef: placement.kernelRef,
  })
  const allSlices = waitingRoomAllSlices(remote)
  const projects = waitingRoomProjectsForNavigation(remote.projects, Boolean(state.showArchivedProjects))
  return {
    session: visibleSessions[state.sessionIndex] ?? null,
    project: projects[state.projectIndex ?? 0] ?? null,
    remoteMachine: remoteMachines[state.machineIndex] ?? null,
    remoteKernel: remoteKernels[state.remoteKernelIndex] ?? null,
    terminal: terminals[state.terminalIndex] ?? null,
    externalProviderSession: externalProviderSessionAtSelection(externalProviderSessions, {
      selectedExternalProviderSessionIndex: state.externalSessionIndex ?? 0,
    }),
    sliceInventory: allSlices[state.sliceIndex ?? 0] ?? null,
    slice: waitingRoomSelectedSlice(state.sliceSelectionId, slices),
    sliceRef: selectedWaitingRoomSliceRef(state.sliceSelectionId, slices),
    sliceCreate: selectedWaitingRoomSliceCreateMode(state.sliceSelectionId, state.sliceDisplayMode),
    machineRef: placement.machineRef,
    kernelRef: placement.kernelRef,
    workerKernelRef: placement.workerKernelRef,
    providerId: state.providerId,
    accountProfile: selectedProviderAccount(
      remote.providerAccounts,
      state.providerId,
      state.accountProfileId,
    ),
    model,
    effort: state.effort,
    projectSelection: projectSelectionFromId(normalizeWaitingRoomProjectSelectionId(
      state.projectSelectionId,
      remote.projects,
      remote.workspaceId,
    )),
    providerCatalogFallback: providerCatalogIsLocalFallback(catalog),
  }
}

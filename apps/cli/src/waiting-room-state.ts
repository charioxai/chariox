import {
  externalProviderSessionPageHasMore,
  externalProviderSessionPageSessions,
  externalProviderSessionSelectionIndex,
} from "@arroba/kernel-client/external-provider-sessions"
import {
  normalizeBackendProviderId,
  selectConfiguredModel,
  selectConfiguredVariant,
  type BackendProviderId,
  type ProviderCatalog,
} from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  DEFAULT_THEME_REGISTRY,
  normalizeThemeName,
  type ThemeRegistry,
} from "./theme-registry.js"
import { waitingRoomEfforts } from "./waiting-room-choice.js"
import {
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachines,
} from "./waiting-room-remote-rows.js"
import { normalizeWaitingRoomLaunchPlacement } from "@arroba/kernel-client/waiting-room-runtime-placement"
import {
  waitingRoomPreviewSessions,
  waitingRoomSessions,
} from "./waiting-room-session-rows.js"
import {
  normalizeWaitingRoomSliceSelection,
  waitingRoomSlices,
} from "./waiting-room-slices.js"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import { normalizeWaitingRoomWorktreeSelectionId } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import {
  DEFAULT_PROJECT_SELECTION_ID,
  normalizeWaitingRoomProjectSelectionId,
} from "./waiting-room-projects.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"

export function createWaitingRoomState(
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  providerId: BackendProviderId,
  model: string,
  effort: string,
  themeId: unknown = "opencode",
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
): WaitingRoomState {
  const selected = selectConfiguredModel(catalog, model, providerId)
  return normalizeWaitingRoomState(
    {
      focus: "new",
      sessionIndex: 0,
      projectIndex: 0,
      externalSessionIndex: 0,
      machineIndex: 0,
      remoteKernelIndex: 0,
      sliceIndex: 0,
      terminalIndex: 0,
      worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(),
      workspaceLiveSyncMode: "off",
      selectedMachineRef: "local",
      selectedKernelRef: "local",
      projectSelectionId: DEFAULT_PROJECT_SELECTION_ID,
      sliceSelectionId: "none",
      sliceDisplayMode: "headless",
      providerId,
      modelId: selected?.id ?? model,
      effort: selectConfiguredVariant(selected, effort),
      executionMode: "build",
      permissionLevel: "yolo",
      themeId: normalizeThemeName(themeId, themeRegistry),
      introStep: 0,
      keyState: { up: false, down: false, left: false, right: false },
    },
    sessions,
    catalog,
    themeRegistry,
  )
}

export function normalizeWaitingRoomState(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  catalog: ProviderCatalog,
  themeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY,
  remote: WaitingRoomRemoteState = {},
): WaitingRoomState {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const allSlices = waitingRoomAllSlices(remote)
  const terminals = waitingRoomTerminals(remote)
  const projects = waitingRoomProjectsForNavigation(remote.projects)
  const externalSessions = externalProviderSessionPageSessions(remote)
  const placement = normalizeWaitingRoomLaunchPlacement(state, remote)
  const slices = waitingRoomSlices(remote, {
    worktreeSelectionId: state.worktreeSelectionId,
    selectedMachineRef: placement.selectedMachineRef,
    selectedKernelRef: placement.selectedKernelRef,
  })
  const providerId = normalizeBackendProvider(state.providerId)
  const selected = selectConfiguredModel(catalog, state.modelId, providerId)
  const efforts = waitingRoomEfforts(selected)
  const sliceSelection = normalizeWaitingRoomSliceSelection(
    state.sliceSelectionId,
    state.sliceDisplayMode,
    slices,
  )
  const focus = (visibleSessions.length === 0 && (state.focus === "session" || state.focus === "join-sessions"))
    ? "new"
    : previewSessions.length === 0 && state.focus === "session"
      ? "join-sessions"
    : externalSessions.length === 0 && state.focus === "external-session"
      ? visibleSessions.length > 0 ? "join-sessions" : "new"
    : state.focus === "external-session-more" && !externalProviderSessionPageHasMore(remote)
      ? externalSessions.length > 0 ? "external-session" : visibleSessions.length > 0 ? "join-sessions" : "new"
    : remoteMachines.length === 0 && state.focus === "machine"
      ? "relay"
        : remoteKernels.length === 0 && state.focus === "remote-kernel"
          ? "relay"
        : allSlices.length === 0 && state.focus === "slice-entry"
          ? "slice"
        : terminals.length === 0 && state.focus === "terminal"
          ? "add-terminal"
        : projects.length === 0 && state.focus === "project-entry"
          ? "new"
        : state.focus === "slice-display"
          ? "slice"
          : state.focus
  return {
    ...state,
    focus,
    providerId,
    sessionIndex: visibleSessions.length === 0 ? 0 : modulo(state.sessionIndex, visibleSessions.length),
    projectIndex: projects.length === 0 ? 0 : modulo(state.projectIndex ?? 0, projects.length),
    externalSessionIndex: externalProviderSessionSelectionIndex(externalSessions, {
      selectedExternalProviderSessionIndex: state.externalSessionIndex ?? null,
    }),
    machineIndex: remoteMachines.length === 0 ? 0 : modulo(state.machineIndex, remoteMachines.length),
    remoteKernelIndex: remoteKernels.length === 0 ? 0 : modulo(state.remoteKernelIndex, remoteKernels.length),
    sliceIndex: allSlices.length === 0 ? 0 : modulo(state.sliceIndex ?? 0, allSlices.length),
    terminalIndex: terminals.length === 0 ? 0 : modulo(state.terminalIndex, terminals.length),
    worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(state.worktreeSelectionId),
    workspaceLiveSyncMode: normalizeWorkspaceLiveSyncMode(state.workspaceLiveSyncMode),
    selectedMachineRef: placement.selectedMachineRef,
    selectedKernelRef: placement.selectedKernelRef,
    projectSelectionId: normalizeWaitingRoomProjectSelectionId(
      state.projectSelectionId,
      remote.projects,
    ),
    ...(sliceSelection.sliceSelectionId !== undefined ? { sliceSelectionId: sliceSelection.sliceSelectionId } : {}),
    ...(sliceSelection.sliceDisplayMode !== undefined ? { sliceDisplayMode: sliceSelection.sliceDisplayMode } : {}),
    modelId: selected?.id ?? state.modelId,
    effort: efforts.includes(state.effort) ? state.effort : efforts[0] ?? "",
    executionMode: waitingRoomExecutionMode(state),
    permissionLevel: waitingRoomPermissionLevel(state),
    themeId: normalizeThemeName(state.themeId, themeRegistry),
  }
}

export function waitingRoomExecutionMode(
  state: Pick<WaitingRoomState, "executionMode">,
): "build" | "plan" {
  return state.executionMode === "plan" ? "plan" : "build"
}

export function waitingRoomPermissionLevel(
  state: Pick<WaitingRoomState, "permissionLevel">,
): "required" | "yolo" {
  return state.permissionLevel === "required" ? "required" : "yolo"
}

function normalizeWorkspaceLiveSyncMode(value: WaitingRoomState["workspaceLiveSyncMode"]): WaitingRoomState["workspaceLiveSyncMode"] {
  return value === "managed" || value === "tracked" ? value : "off"
}

function normalizeBackendProvider(value: string): BackendProviderId {
  return normalizeBackendProviderId(value)
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

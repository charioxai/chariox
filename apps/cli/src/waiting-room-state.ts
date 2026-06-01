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
import {
  waitingRoomPreviewSessions,
  waitingRoomSessions,
} from "./waiting-room-session-rows.js"
import {
  normalizeWaitingRoomSliceSelectionId,
  waitingRoomSlices,
} from "./waiting-room-slices.js"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import { normalizeWaitingRoomWorktreeSelectionId } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

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
      machineIndex: 0,
      remoteKernelIndex: 0,
      sliceIndex: 0,
      terminalIndex: 0,
      worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(),
      workspaceLiveSyncMode: "off",
      sliceSelectionId: "none",
      sliceDisplayMode: "headless",
      providerId,
      modelId: selected?.id ?? model,
      effort: selectConfiguredVariant(selected, effort),
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
  const slices = waitingRoomSlices(remote, { worktreeSelectionId: state.worktreeSelectionId })
  const providerId = normalizeBackendProvider(state.providerId)
  const selected = selectConfiguredModel(catalog, state.modelId, providerId)
  const efforts = waitingRoomEfforts(selected)
  const focus = (visibleSessions.length === 0 && (state.focus === "session" || state.focus === "join-sessions"))
    ? "new"
    : previewSessions.length === 0 && state.focus === "session"
      ? "join-sessions"
    : remoteMachines.length === 0 && state.focus === "machine"
      ? "relay"
        : remoteKernels.length === 0 && state.focus === "remote-kernel"
          ? "relay"
        : allSlices.length === 0 && state.focus === "slice-entry"
          ? "slice"
        : terminals.length === 0 && state.focus === "terminal"
          ? "add-terminal"
        : state.focus
  return {
    ...state,
    focus,
    providerId,
    sessionIndex: visibleSessions.length === 0 ? 0 : modulo(state.sessionIndex, visibleSessions.length),
    machineIndex: remoteMachines.length === 0 ? 0 : modulo(state.machineIndex, remoteMachines.length),
    remoteKernelIndex: remoteKernels.length === 0 ? 0 : modulo(state.remoteKernelIndex, remoteKernels.length),
    sliceIndex: allSlices.length === 0 ? 0 : modulo(state.sliceIndex ?? 0, allSlices.length),
    terminalIndex: terminals.length === 0 ? 0 : modulo(state.terminalIndex, terminals.length),
    worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(state.worktreeSelectionId),
    workspaceLiveSyncMode: normalizeWorkspaceLiveSyncMode(state.workspaceLiveSyncMode),
    sliceSelectionId: normalizeWaitingRoomSliceSelectionId(state.sliceSelectionId, slices),
    sliceDisplayMode: normalizeSliceDisplayMode(state.sliceDisplayMode),
    modelId: selected?.id ?? state.modelId,
    effort: efforts.includes(state.effort) ? state.effort : efforts[0] ?? "",
    themeId: normalizeThemeName(state.themeId, themeRegistry),
  }
}

function normalizeSliceDisplayMode(value: WaitingRoomState["sliceDisplayMode"]): NonNullable<WaitingRoomState["sliceDisplayMode"]> {
  return value === "headed" ? "headed" : "headless"
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

import {
  externalProviderSessionPageHasMore,
  externalProviderSessionPageSessions,
  externalProviderSessionSelectionIndex,
} from "@chariox/kernel-client/external-provider-sessions"
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
import { normalizeWaitingRoomLaunchPlacement } from "@chariox/kernel-client/waiting-room-runtime-placement"
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
  normalizeWaitingRoomProjectSelectionId,
} from "./waiting-room-projects.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import {
  managedProviderAccountIsTransferable,
  normalizeWaitingRoomManagedDraft,
  waitingRoomConfiguresNewManagedMachine,
  waitingRoomProjectRepositoryOptions,
} from "./waiting-room-managed-environments.js"
import { providerAccountsForProvider } from "./waiting-room-provider-accounts.js"

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
      externalSessionIndex: 0,
      machineIndex: 0,
      remoteKernelIndex: 0,
      sliceIndex: 0,
      terminalIndex: 0,
      worktreeSelectionId: normalizeWaitingRoomWorktreeSelectionId(),
      workspaceLiveSyncMode: "off",
      selectedMachineRef: "local",
      selectedKernelRef: "local",
      managedKernelContext: "empty",
      managedDevelopmentMode: "empty",
      managedRepositoryIndex: 0,
      managedProviderAccountSource: "none",
      managedProviderAccountIndex: 0,
      managedGitCredentialSource: "none",
      managedAutoStopPreset: "idle_15m",
      managedCustomMinimumRuntimeSeconds: 0,
      managedCustomIdleDelaySeconds: 900,
      sliceSelectionId: "none",
      sliceDisplayMode: "headless",
      providerId,
      accountProfileId: "default",
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
  const projects = waitingRoomProjectsForNavigation(remote.projects, Boolean(state.showArchivedProjects))
  const externalSessions = externalProviderSessionPageSessions(remote)
  const placement = normalizeWaitingRoomLaunchPlacement(state, remote)
  const slices = waitingRoomSlices(remote, {
    worktreeSelectionId: state.worktreeSelectionId,
    projectSelectionId: state.projectSelectionId,
    developmentMode: state.managedDevelopmentMode,
    repositorySelection: state.managedRepositorySelection,
    selectedMachineRef: placement.selectedMachineRef,
    selectedKernelRef: placement.selectedKernelRef,
  })
  const providerId = normalizeBackendProvider(state.providerId)
  const providerAccounts = providerAccountsForProvider(remote.providerAccounts, providerId)
  const accountProfileId = providerAccounts.some((profile) => profile.profile_id === state.accountProfileId)
    ? state.accountProfileId ?? "default"
    : providerAccounts.find((profile) => profile.is_default)?.profile_id
      ?? providerAccounts[0]?.profile_id
      ?? "default"
  const selected = selectConfiguredModel(catalog, state.modelId, providerId)
  const efforts = waitingRoomEfforts(selected)
  const sliceSelection = normalizeWaitingRoomSliceSelection(
    state.sliceSelectionId,
    state.sliceDisplayMode,
    slices,
  )
  const configuresManaged = waitingRoomConfiguresNewManagedMachine(placement.selectedMachineRef)
  const configuresSliceDevelopment = !configuresManaged
    && Boolean(sliceSelection.sliceSelectionId && sliceSelection.sliceSelectionId !== "none")
  const sliceDevelopmentFocus = state.focus === "managed-development"
    || state.focus === "managed-repositories"
  const focus = (state.focus.startsWith("managed-")
    && !configuresManaged
    && !(configuresSliceDevelopment && sliceDevelopmentFocus))
    ? sliceDevelopmentFocus ? "slice" : "launch-machine"
    : (visibleSessions.length === 0 && (state.focus === "session" || state.focus === "join-sessions"))
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
  const normalized = normalizeWaitingRoomManagedDraft({
    ...state,
    focus,
    providerId,
    accountProfileId,
    sessionIndex: visibleSessions.length === 0 ? 0 : modulo(state.sessionIndex, visibleSessions.length),
    projectIndex: projects.length === 0 ? 0 : modulo(state.projectIndex ?? 0, projects.length),
    showArchivedProjects: Boolean(state.showArchivedProjects),
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
    ...((state.projectSelectionId !== undefined || remote.projects !== undefined)
      ? {
          projectSelectionId: normalizeWaitingRoomProjectSelectionId(
            state.projectSelectionId,
            remote.projects,
            remote.workspaceId,
          ),
        }
      : {}),
    ...(sliceSelection.sliceSelectionId !== undefined ? { sliceSelectionId: sliceSelection.sliceSelectionId } : {}),
    ...(sliceSelection.sliceDisplayMode !== undefined ? { sliceDisplayMode: sliceSelection.sliceDisplayMode } : {}),
    modelId: selected?.id ?? state.modelId,
    effort: efforts.includes(state.effort) ? state.effort : efforts[0] ?? "",
    executionMode: waitingRoomExecutionMode(state),
    permissionLevel: waitingRoomPermissionLevel(state),
    themeId: normalizeThemeName(state.themeId, themeRegistry),
  }, remote)
  const selectedRepositoryTargetExists = normalized.managedDevelopmentMode === "current_project"
    && waitingRoomProjectRepositoryOptions(normalized, remote).length > 1
  if (normalized.focus === "managed-repositories" && !selectedRepositoryTargetExists) {
    return { ...normalized, focus: "managed-development" }
  }
  if (normalized.focus === "managed-provider-account") {
    const focusedProviderAccount = remote.providerAccounts?.[normalized.managedProviderAccountIndex ?? 0]
    if (!focusedProviderAccount || !managedProviderAccountIsTransferable(focusedProviderAccount)) {
      return { ...normalized, focus: "managed-provider-accounts" }
    }
  }
  return normalized
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

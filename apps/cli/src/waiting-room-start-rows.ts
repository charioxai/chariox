import {
  backendProviderLabel,
  type BackendProviderId,
  type CatalogModelOption,
} from "./provider-catalog.js"
import type { SliceRecord } from "./cli-types.js"
import type { ProviderAccountProfile } from "@chariox/kernel-client"
import { formatWaitingRoomSliceSelection, waitingRoomSlices } from "./waiting-room-slices.js"
import {
  formatWaitingRoomLaunchKernelValue,
  formatWaitingRoomLaunchMachineValue,
  waitingRoomLaunchKernelOptions,
  waitingRoomLaunchMachineOptions,
  waitingRoomSelectedLaunchKernelRef,
  waitingRoomSelectedLaunchMachineRef,
} from "@chariox/kernel-client/waiting-room-runtime-placement"
import { describeWaitingRoomWorktreeSelection } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState, WaitingRoomTargetState } from "./waiting-room-types.js"
import { describeWaitingRoomProjectSelection } from "./waiting-room-projects.js"
import {
  managedAutoStopLabel,
  managedDurationLabel,
  managedEnvironmentIsLaunchReady,
  managedKernelContextLabel,
  selectedManagedEnvironment,
  waitingRoomConfiguresNewManagedMachine,
} from "./waiting-room-managed-environments.js"

export type WaitingRoomStartRowsChoice = {
  providerId: BackendProviderId
  model: CatalogModelOption | null
  effort: string
  accountProfile?: ProviderAccountProfile | null
  slice?: SliceRecord | null
  providerCatalogFallback?: boolean
}

export function waitingRoomStartRows(
  state: Pick<WaitingRoomState,
    | "focus"
    | "worktreeSelectionId"
    | "workspaceLiveSyncMode"
    | "selectedMachineRef"
    | "selectedKernelRef"
    | "projectSelectionId"
    | "sliceSelectionId"
    | "sliceDisplayMode"
    | "managedComputeClass"
    | "managedRegion"
    | "managedKernelContext"
    | "managedContextSourceTargetId"
    | "managedDevelopmentMode"
    | "managedRepositoryMode"
    | "managedProviderAccountSource"
    | "managedGitCredentialSource"
    | "managedAutoStopPreset"
    | "managedCustomMinimumRuntimeSeconds"
    | "managedCustomIdleDelaySeconds"
  >,
  choice: WaitingRoomStartRowsChoice,
  options: {
    modelOptions: CatalogModelOption[]
    remote?: WaitingRoomRemoteState
    targets?: WaitingRoomTargetState
    inventoryLoading: boolean
    loadingText: string
    visibleSessionCount: number
    titleWidth: number
  },
): WaitingRoomRow[] {
  const remote = options.remote ?? {}
  const selectedWorktreeLabel = describeWaitingRoomWorktreeSelection(
    state.worktreeSelectionId,
    options.targets?.worktreePath,
  )
  const selectedSliceLabel = formatWaitingRoomSliceSelection(
    state.sliceSelectionId,
    waitingRoomSlices(remote, {
      workspacePath: options.targets?.workspacePath,
      worktreeSelectionId: state.worktreeSelectionId,
      worktreePath: options.targets?.worktreePath,
      selectedMachineRef: state.selectedMachineRef,
      selectedKernelRef: state.selectedKernelRef,
    }),
    state.sliceDisplayMode,
  )
  const collaborationBackend = remote.collaborationBackend ?? "local"
  const configuresManaged = waitingRoomConfiguresNewManagedMachine(state.selectedMachineRef)
  const selectedEnvironment = selectedManagedEnvironment(state, remote)
  const managedRows: WaitingRoomRow[] = configuresManaged
    ? [
        startRow("managed-compute", "Compute class", state.managedComputeClass ?? "Unavailable", state, options.titleWidth),
        startRow("managed-region", "Region", state.managedRegion ?? "Unavailable", state, options.titleWidth),
        startRow("managed-kernel-context", "Kernel context from", managedKernelContextLabel(state as WaitingRoomState, remote), state, options.titleWidth),
        startRow("managed-development", "Development setup", state.managedDevelopmentMode === "empty" ? "Empty" : "Current Project", state, options.titleWidth),
        startRow(
          "managed-repositories",
          "Selected repositories",
          state.managedDevelopmentMode === "empty"
            ? "None"
            : state.managedRepositoryMode === "project_defaults" ? "Project defaults" : "Primary only",
          state,
          options.titleWidth,
        ),
        startRow(
          "managed-provider-accounts",
          "Provider accounts source",
          state.managedProviderAccountSource === "none" ? "None" : "Selected account",
          state,
          options.titleWidth,
        ),
        startRow(
          "managed-git-credentials",
          "Git credentials source",
          state.managedGitCredentialSource === "none" ? "None" : "GitHub",
          state,
          options.titleWidth,
        ),
        startRow("managed-auto-stop", "Auto-stop policy", managedAutoStopLabel(state as WaitingRoomState), state, options.titleWidth),
        ...(state.managedAutoStopPreset === "custom"
          ? [
              startRow(
                "managed-custom-minimum",
                "Minimum runtime",
                managedDurationLabel(state.managedCustomMinimumRuntimeSeconds),
                state,
                options.titleWidth,
              ),
              startRow(
                "managed-custom-idle",
                "Idle delay",
                managedDurationLabel(state.managedCustomIdleDelaySeconds),
                state,
                options.titleWidth,
              ),
            ]
          : []),
      ]
    : []
  return [
    {
      id: "new",
      title: configuresManaged
        ? "Create machine and start session"
        : selectedEnvironment && !managedEnvironmentIsLaunchReady(selectedEnvironment)
          ? "Start machine and session"
          : "Start New Session",
      value: "Press Enter",
      titleWidth: options.titleWidth,
      indent: 0,
      focused: state.focus === "new",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "launch-machine",
      title: "Machine",
      value: formatWaitingRoomLaunchMachineValue(state, remote),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "launch-machine",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "launch-kernel",
      title: "Kernel",
      value: formatWaitingRoomLaunchKernelValue(state, remote),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "launch-kernel",
      selectable: true,
      scrollbar: "",
    },
    ...managedRows,
    ...((state.projectSelectionId !== undefined || remote.projects !== undefined)
      ? [{
          id: "project",
          title: "Project",
          value: describeWaitingRoomProjectSelection(
            state.projectSelectionId ?? "default",
            remote.projects,
            options.targets?.workspacePath,
          ),
          titleWidth: options.titleWidth,
          indent: 1,
          focused: state.focus === "project",
          selectable: true,
          scrollbar: "",
        }]
      : []),
    {
      id: "provider",
      title: "Provider",
      value: formatProviderValue(
        choice.providerId,
        choice.providerCatalogFallback,
      ),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "provider",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "account",
      title: "Account",
      value: formatAccountValue(choice.accountProfile ?? null),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "account",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "model",
      title: "Model",
      value: choice.model ? formatWaitingRoomModelValue(choice.model, options.modelOptions, choice.providerCatalogFallback) : "No models available",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "model",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "effort",
      title: "Variant",
      value: formatVariantValue(choice.effort, choice.providerCatalogFallback),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "effort",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "workspace",
      title: "Workspace",
      value: options.targets?.workspacePath ?? (options.inventoryLoading ? options.loadingText : "Set workspace path"),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "workspace",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "worktree",
      title: "Worktree",
      value: options.targets?.worktreePath
        ? selectedWorktreeLabel
        : options.inventoryLoading
          ? options.loadingText
          : selectedWorktreeLabel,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "worktree",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "live-sync",
      title: "Live Sync",
      value: formatWorkspaceLiveSyncMode(state.workspaceLiveSyncMode),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "live-sync",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "collaborators",
      title: "Collaborators",
      value: collaborationBackend === "cloud" ? "use Cloud" : "after session start",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "collaborators",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "slice",
      title: "Slice",
      value: options.inventoryLoading ? options.loadingText : selectedSliceLabel,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "slice",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "join-header",
      title: "Join Existing Session",
      value: options.inventoryLoading && options.visibleSessionCount === 0
        ? options.loadingText
        : options.visibleSessionCount > 0
          ? "Press Enter"
          : "",
      titleWidth: options.titleWidth,
      indent: 0,
      focused: state.focus === "join-sessions",
      selectable: true,
      scrollbar: "",
    },
  ]
}

function startRow(
  id: WaitingRoomState["focus"],
  title: string,
  value: string,
  state: Pick<WaitingRoomState, "focus">,
  titleWidth: number,
): WaitingRoomRow {
  return {
    id,
    title,
    value,
    titleWidth,
    indent: 1,
    focused: state.focus === id,
    selectable: true,
    scrollbar: "",
  }
}

function formatAccountValue(profile: ProviderAccountProfile | null): string {
  if (!profile) {
    return "Default (not discovered)"
  }
  const identity = profile.identity_summary ? ` · ${profile.identity_summary}` : ""
  const usage = compactUsage(profile)
  return `${profile.label}${identity}${usage ? ` · ${usage}` : ""}`
}

function compactUsage(profile: ProviderAccountProfile): string | null {
  const meters = profile.usage.meters ?? []
  const meter = meters.find((candidate) => candidate.state === "exhausted") ?? meters[0]
  if (!meter) {
    return profile.usage.availability === "unavailable" ? "usage not observed" : null
  }
  if (meter.used_percent !== undefined && meter.used_percent !== null) {
    return `${Math.round(meter.used_percent)}% used`
  }
  if (meter.remaining !== undefined && meter.remaining !== null) {
    return `${meter.remaining}${meter.unit ? ` ${meter.unit}` : ""} remaining`
  }
  return meter.label
}

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function formatBackendProviderLabel(providerId: BackendProviderId) {
  return backendProviderLabel(providerId)
}

function formatProviderValue(
  providerId: BackendProviderId,
  fallback = false,
) {
  const label = formatBackendProviderLabel(providerId)
  return fallback ? `${label} (local list)` : label
}

function formatWaitingRoomModelValue(
  model: CatalogModelOption,
  options: CatalogModelOption[],
  fallback = false,
) {
  const label = formatWaitingRoomModelLabel(model, options)
  return fallback ? `${label} (local list)` : label
}

function formatVariantValue(effort: string, fallback = false) {
  const label = effort ? formatTitleCase(effort) : "Default"
  return fallback ? `${label} (local list)` : label
}

function formatWorkspaceLiveSyncMode(mode: WaitingRoomState["workspaceLiveSyncMode"]) {
  if (mode === "managed") return "managed (selected workspace/worktree only; other repositories unrestricted)"
  if (mode === "tracked") return "tracked (turn-end; selected workspace/worktree only; other repositories unrestricted)"
  return "off (default; all repositories unrestricted)"
}

function formatWaitingRoomModelLabel(
  model: CatalogModelOption,
  options: CatalogModelOption[],
) {
  const providerCount = new Set(options.map((option) => option.providerId)).size
  return providerCount <= 1 ? model.label : `${model.providerName} ${model.label}`
}

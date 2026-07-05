import {
  backendProviderLabel,
  type BackendProviderId,
  type CatalogModelOption,
} from "./provider-catalog.js"
import {
  formatProviderAccountForBackend,
  formatSliceBackendProviderAccount,
} from "./slice-format.js"
import type { SliceRecord } from "./cli-types.js"
import { formatWaitingRoomSliceSelection, waitingRoomSlices } from "./waiting-room-slices.js"
import {
  formatWaitingRoomLaunchKernelValue,
  formatWaitingRoomLaunchMachineValue,
  waitingRoomLaunchKernelOptions,
  waitingRoomLaunchMachineOptions,
  waitingRoomSelectedLaunchKernelRef,
  waitingRoomSelectedLaunchMachineRef,
} from "@arroba/kernel-client/waiting-room-runtime-placement"
import { describeWaitingRoomWorktreeSelection } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState, WaitingRoomTargetState } from "./waiting-room-types.js"

export type WaitingRoomStartRowsChoice = {
  providerId: BackendProviderId
  model: CatalogModelOption | null
  effort: string
  slice?: SliceRecord | null
  providerCatalogFallback?: boolean
}

export function waitingRoomStartRows(
  state: Pick<WaitingRoomState, "focus" | "worktreeSelectionId" | "workspaceLiveSyncMode" | "selectedMachineRef" | "selectedKernelRef" | "sliceSelectionId" | "sliceDisplayMode">,
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
  return [
    {
      id: "new",
      title: "Start New Session",
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
    {
      id: "provider",
      title: "Provider",
      value: formatProviderValue(
        choice.providerId,
        choice.slice ?? null,
        state,
        remote,
        choice.providerCatalogFallback,
      ),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "provider",
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

function formatTitleCase(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function formatBackendProviderLabel(providerId: BackendProviderId) {
  return backendProviderLabel(providerId)
}

function formatProviderValue(
  providerId: BackendProviderId,
  slice: SliceRecord | null,
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomRemoteState,
  fallback = false,
) {
  const label = formatBackendProviderLabel(providerId)
  const account = slice
    ? formatSliceBackendProviderAccount(slice, providerId)
    : formatRemoteProviderAccount(providerId, state, remote)
  const providerLabel = account ? `${label} (${account})` : label
  return fallback ? `${providerLabel} (local list)` : providerLabel
}

function formatRemoteProviderAccount(
  providerId: BackendProviderId,
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomRemoteState,
): string | null {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const kernelRef = waitingRoomSelectedLaunchKernelRef({ ...state, selectedMachineRef: machineRef }, remote)
  const kernel = waitingRoomLaunchKernelOptions(remote, machineRef)
    .find((option) => option.id === kernelRef)
    ?.kernel
  const kernelAccount = formatProviderAccountForBackend(
    kernel?.provider_accounts ?? [],
    providerId,
    kernel?.available_providers ?? [],
  )
  if (kernelAccount) {
    return kernelAccount
  }
  const machine = waitingRoomLaunchMachineOptions(remote)
    .find((option) => option.id === machineRef)
    ?.machine
  return formatProviderAccountForBackend(
    machine?.provider_accounts ?? [],
    providerId,
    machine?.available_providers ?? [],
  )
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

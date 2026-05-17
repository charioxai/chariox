import {
  backendProviderLabel,
  type BackendProviderId,
  type CatalogModelOption,
} from "./provider-catalog.js"
import { formatWaitingRoomSliceSelection, waitingRoomSlices } from "./waiting-room-slices.js"
import { describeWaitingRoomWorktreeSelection } from "./waiting-room-worktrees.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState, WaitingRoomTargetState } from "./waiting-room.js"

export type WaitingRoomStartRowsChoice = {
  providerId: BackendProviderId
  model: CatalogModelOption | null
  effort: string
}

export function waitingRoomStartRows(
  state: Pick<WaitingRoomState, "focus" | "worktreeSelectionId" | "sliceSelectionId">,
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
  const selectedSliceLabel = formatWaitingRoomSliceSelection(state.sliceSelectionId, waitingRoomSlices(remote))
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
      id: "provider",
      title: "Provider",
      value: formatBackendProviderLabel(choice.providerId),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "provider",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "model",
      title: "Model",
      value: choice.model ? formatWaitingRoomModelLabel(choice.model, options.modelOptions) : "No models available",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "model",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "effort",
      title: "Variant",
      value: choice.effort ? formatTitleCase(choice.effort) : "Default",
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

function formatWaitingRoomModelLabel(
  model: CatalogModelOption,
  options: CatalogModelOption[],
) {
  const providerCount = new Set(options.map((option) => option.providerId)).size
  return providerCount <= 1 ? model.label : `${model.providerName} ${model.label}`
}

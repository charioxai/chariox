import type { SliceRecord } from "./cli-types.js"
import {
  formatSliceProviderAuth,
  formatSliceProviderList,
  formatSliceRelayLabel,
  sliceProviderAuthCoverage,
} from "./slice-format.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { selectedWaitingRoomWorktreePath } from "./waiting-room-worktrees.js"

export type WaitingRoomSliceScope = {
  workspacePath?: string | null | undefined
  worktreeSelectionId?: string | null | undefined
  worktreePath?: string | null | undefined
  selectedMachineRef?: string | null | undefined
  selectedKernelRef?: string | null | undefined
}

export function waitingRoomSlices(
  remote: { slices?: SliceRecord[] } = {},
  scope: WaitingRoomSliceScope = {},
) {
  const worktreePath = selectedWaitingRoomWorktreePath(scope.worktreeSelectionId, scope.worktreePath)
  return (remote.slices ?? [])
    .filter((slice) => sliceCompatibleWithScope(slice, scope.workspacePath, worktreePath, scope.selectedMachineRef, scope.selectedKernelRef))
    .slice()
    .sort((left, right) => formatWaitingRoomSliceLabel(left).localeCompare(formatWaitingRoomSliceLabel(right)))
}

export function waitingRoomSliceOptions(slices: SliceRecord[]) {
  return [
    { id: "none", slice: null },
    { id: "new:headless", slice: null },
    { id: "new:headed", slice: null },
    ...slices.map((slice) => ({ id: slice.id, slice })),
  ]
}

export function normalizeWaitingRoomSliceSelectionId(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  if (normalized === "none") {
    return "none"
  }
  if (normalized === "new" || normalized === "new:headless" || normalized === "new:headed") {
    return "new"
  }
  const slice = waitingRoomSelectedSlice(normalized, slices)
  return slice?.id ?? normalized
}

export function waitingRoomSelectedSlice(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "none" || selectionId === "new") {
    return null
  }
  return slices.find((slice) => slice.id === selectionId || slice.name === selectionId) ?? null
}

export function selectedWaitingRoomSliceRef(selectionId: string | null | undefined, slices: SliceRecord[]) {
  return waitingRoomSelectedSlice(selectionId, slices)?.id ?? null
}

export function selectedWaitingRoomSliceCreateMode(
  selectionId: string | null | undefined,
  displayMode: "headless" | "headed" | null | undefined,
) {
  return selectionId === "new"
    ? { displayMode: displayMode === "headed" ? "headed" as const : "headless" as const }
    : null
}

export function waitingRoomSliceSelectionUnavailable(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  return normalized !== "none"
    && normalized !== "new"
    && normalized !== "new:headless"
    && normalized !== "new:headed"
    && !waitingRoomSelectedSlice(normalized, slices)
}

export function formatWaitingRoomSliceSelection(
  selectionId: string | null | undefined,
  slices: SliceRecord[],
  displayMode: "headless" | "headed" | null | undefined = "headless",
) {
  if (selectionId === "new") {
    return displayMode === "headed" ? "new headed" : "new headless"
  }
  const slice = waitingRoomSelectedSlice(selectionId, slices)
  if (slice) {
    return formatWaitingRoomSliceOption(slice)
  }
  return waitingRoomSliceSelectionUnavailable(selectionId, slices) ? "reuse unavailable" : "off"
}

export function formatWaitingRoomSliceLabel(slice: SliceRecord) {
  return slice.name || slice.id
}

export function formatWaitingRoomSliceOption(slice: SliceRecord) {
  const agents = slice.agent_ids?.length ?? 0
  const auth = waitingRoomSliceAuthDetail(slice)
  const relay = formatSliceRelayLabel(slice)
  const failed = slice.last_operation_status === "failed"
    ? `, error ${slice.last_error ?? slice.last_operation ?? "failed"}`
    : ""
  return `${formatWaitingRoomSliceLabel(slice)} (${[
    slice.status,
    slice.display_mode ?? "headless",
    `${agents} agent${agents === 1 ? "" : "s"}`,
    relay ? `relay ${relay}` : "",
    auth,
  ].filter(Boolean).join(", ")}${failed})`
}

function waitingRoomSliceAuthDetail(slice: SliceRecord): string {
  const authEntries = slice.provider_auth ?? []
  if (authEntries.length === 0) {
    const coverage = sliceProviderAuthCoverage(slice)
    return coverage.providers.length > 0
      ? `auth missing ${formatSliceProviderList(coverage.providers)}`
      : "providers missing"
  }
  const coverage = sliceProviderAuthCoverage(slice)
  const details = authEntries
    .map((entry) => formatSliceProviderAuth(entry, {
      separator: " ",
      includeOrgPlan: false,
    }))
    .filter(Boolean)
  if (coverage.missingProviders.length > 0) {
    details.push(`missing ${formatSliceProviderList(coverage.missingProviders)}`)
  }
  if (coverage.staleProviders.length > 0) {
    details.push(`refresh ${formatSliceProviderList(coverage.staleProviders)}`)
  }
  return details.join(", ")
}

export function cycleWaitingRoomSliceSelectionId(
  selectionId: string | null | undefined,
  slices: SliceRecord[],
  delta: number,
) {
  const options = waitingRoomSliceOptions(slices)
  const currentId = selectionId === "new" ? "new:headless" : selectionId
  const index = Math.max(0, options.findIndex((option) => option.id === currentId))
  return options[modulo(index + delta, options.length)]?.id ?? "none"
}

export function cycleWaitingRoomSliceSelection(
  state: WaitingRoomState,
  slices: SliceRecord[],
  delta: number,
): WaitingRoomState {
  const currentId = state.sliceSelectionId === "new"
    ? state.sliceDisplayMode === "headed" ? "new:headed" : "new:headless"
    : state.sliceSelectionId ?? "none"
  const nextId = cycleWaitingRoomSliceSelectionId(currentId, slices, delta)
  if (nextId === "new:headed" || nextId === "new:headless") {
    return {
      ...state,
      sliceSelectionId: "new",
      sliceDisplayMode: nextId === "new:headed" ? "headed" : "headless",
    }
  }
  return {
    ...state,
    sliceSelectionId: nextId,
  }
}

function sliceCompatibleWithScope(
  slice: SliceRecord,
  workspacePath: string | null | undefined,
  worktreePath: string,
  machineRef: string | null | undefined,
  kernelRef: string | null | undefined,
) {
  if (slice.workspace_id && workspacePath && slice.workspace_id !== workspacePath) {
    return false
  }
  const sliceWorktree = slice.worktree_id || slice.workspace_mount || ""
  if (sliceWorktree && worktreePath && sliceWorktree !== worktreePath) {
    return false
  }
  const selectedKernelRef = kernelRef?.trim()
  const selectedMachineRef = machineRef?.trim()
  if (selectedKernelRef && selectedKernelRef !== "local") {
    return slice.worker_kernel_id === selectedKernelRef || slice.worker_kernel_ref === selectedKernelRef
  }
  if (selectedMachineRef && selectedMachineRef !== "local") {
    return slice.worker_machine_id === selectedMachineRef
  }
  return true
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

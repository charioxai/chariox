import type { SliceRecord } from "./cli-types.js"
import { formatSliceProviderAuth, formatSliceRelayLabel } from "./slice-format.js"
import { selectedWaitingRoomWorktreePath } from "./waiting-room-worktrees.js"

export type WaitingRoomSliceScope = {
  workspacePath?: string | null | undefined
  worktreeSelectionId?: string | null | undefined
  worktreePath?: string | null | undefined
}

export function waitingRoomSlices(
  remote: { slices?: SliceRecord[] } = {},
  scope: WaitingRoomSliceScope = {},
) {
  const worktreePath = selectedWaitingRoomWorktreePath(scope.worktreeSelectionId, scope.worktreePath)
  return (remote.slices ?? [])
    .filter((slice) => sliceCompatibleWithScope(slice, scope.workspacePath, worktreePath))
    .slice()
    .sort((left, right) => formatWaitingRoomSliceLabel(left).localeCompare(formatWaitingRoomSliceLabel(right)))
}

export function waitingRoomSliceOptions(slices: SliceRecord[]) {
  return [
    { id: "none", slice: null },
    { id: "new", slice: null },
    ...slices.map((slice) => ({ id: slice.id, slice })),
  ]
}

export function normalizeWaitingRoomSliceSelectionId(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  if (normalized === "none" || normalized === "new") {
    return normalized
  }
  return waitingRoomSliceOptions(slices).some((option) => option.id === normalized)
    ? normalized
    : "none"
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

export function formatWaitingRoomSliceSelection(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "new") {
    return "new"
  }
  const slice = waitingRoomSelectedSlice(selectionId, slices)
  return slice ? formatWaitingRoomSliceOption(slice) : "off"
}

export function formatWaitingRoomSliceLabel(slice: SliceRecord) {
  return slice.name || slice.id
}

export function formatWaitingRoomSliceOption(slice: SliceRecord) {
  const agents = slice.agent_ids?.length ?? 0
  const auth = (slice.provider_auth ?? [])
    .map((entry) => formatSliceProviderAuth(entry, {
      separator: " ",
      includeOrgPlan: false,
    }))
    .filter(Boolean)
    .join(", ")
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

export function cycleWaitingRoomSliceSelectionId(
  selectionId: string | null | undefined,
  slices: SliceRecord[],
  delta: number,
) {
  const options = waitingRoomSliceOptions(slices)
  const index = Math.max(0, options.findIndex((option) => option.id === selectionId))
  return options[modulo(index + delta, options.length)]?.id ?? "none"
}

function sliceCompatibleWithScope(slice: SliceRecord, workspacePath: string | null | undefined, worktreePath: string) {
  if (slice.workspace_id && workspacePath && slice.workspace_id !== workspacePath) {
    return false
  }
  const sliceWorktree = slice.worktree_id || slice.workspace_mount || ""
  if (sliceWorktree && worktreePath && sliceWorktree !== worktreePath) {
    return false
  }
  return true
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

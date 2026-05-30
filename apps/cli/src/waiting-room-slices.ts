import type { SliceRecord } from "./cli-types.js"
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
    { id: "new:headless", slice: null },
    { id: "new:headed", slice: null },
    ...slices.map((slice) => ({ id: slice.id, slice })),
  ]
}

export function normalizeWaitingRoomSliceSelectionId(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  if (normalized === "none" || normalized === "new:headless" || normalized === "new:headed") {
    return normalized
  }
  return waitingRoomSliceOptions(slices).some((option) => option.id === normalized)
    ? normalized
    : "none"
}

export function waitingRoomSelectedSlice(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "none" || selectionId?.startsWith("new:")) {
    return null
  }
  return slices.find((slice) => slice.id === selectionId || slice.name === selectionId) ?? null
}

export function selectedWaitingRoomSliceRef(selectionId: string | null | undefined, slices: SliceRecord[]) {
  return waitingRoomSelectedSlice(selectionId, slices)?.id ?? null
}

export function selectedWaitingRoomSliceCreateMode(selectionId: string | null | undefined) {
  return selectionId === "new:headless" || selectionId === "new:headed"
    ? { displayMode: selectionId.slice("new:".length) as "headless" | "headed" }
    : null
}

export function formatWaitingRoomSliceSelection(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "new:headless") {
    return "New headless"
  }
  if (selectionId === "new:headed") {
    return "New headed"
  }
  const slice = waitingRoomSelectedSlice(selectionId, slices)
  return slice ? formatWaitingRoomSliceOption(slice) : "None"
}

export function formatWaitingRoomSliceLabel(slice: SliceRecord) {
  return slice.name || slice.id
}

export function formatWaitingRoomSliceOption(slice: SliceRecord) {
  const agents = slice.agent_ids?.length ?? 0
  const auth = (slice.provider_auth ?? [])
    .map((entry) => entry.alias || entry.email || entry.account_id || entry.auth_type || entry.state)
    .filter(Boolean)
    .join(",")
  return `${formatWaitingRoomSliceLabel(slice)} (${agents} agent${agents === 1 ? "" : "s"}${auth ? `, ${auth}` : ""})`
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

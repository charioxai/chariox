import type { SliceRecord } from "./cli-types.js"

export function waitingRoomSlices(remote: { slices?: SliceRecord[] } = {}) {
  return (remote.slices ?? [])
    .slice()
    .sort((left, right) => formatWaitingRoomSliceLabel(left).localeCompare(formatWaitingRoomSliceLabel(right)))
}

export function waitingRoomSliceOptions(slices: SliceRecord[]) {
  return [
    { id: "none", slice: null },
    ...slices.map((slice) => ({ id: slice.id, slice })),
  ]
}

export function normalizeWaitingRoomSliceSelectionId(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const normalized = selectionId?.trim() || "none"
  if (normalized === "none") {
    return "none"
  }
  return waitingRoomSliceOptions(slices).some((option) => option.id === normalized)
    ? normalized
    : "none"
}

export function waitingRoomSelectedSlice(selectionId: string | null | undefined, slices: SliceRecord[]) {
  if (selectionId === "none") {
    return null
  }
  return slices.find((slice) => slice.id === selectionId || slice.name === selectionId) ?? null
}

export function selectedWaitingRoomSliceRef(selectionId: string | null | undefined, slices: SliceRecord[]) {
  return waitingRoomSelectedSlice(selectionId, slices)?.id ?? null
}

export function formatWaitingRoomSliceSelection(selectionId: string | null | undefined, slices: SliceRecord[]) {
  const slice = waitingRoomSelectedSlice(selectionId, slices)
  return slice ? formatWaitingRoomSliceLabel(slice) : "None"
}

export function formatWaitingRoomSliceLabel(slice: SliceRecord) {
  return slice.name || slice.id
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

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

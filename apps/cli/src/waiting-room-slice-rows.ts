import type { SliceRecord } from "./cli-types.js"
import { formatSliceProviderAuth, formatSliceRelayLabel, formatSliceScope } from "./slice-format.js"
import { formatWaitingRoomSliceLabel } from "./waiting-room-slices.js"
import type { WaitingRoomRemoteState, WaitingRoomRow, WaitingRoomState } from "./waiting-room-types.js"

export function waitingRoomSliceRows(
  state: Pick<WaitingRoomState, "focus" | "sliceIndex">,
  remote: Pick<WaitingRoomRemoteState, "inventoryStatus" | "loadingFrame" | "slices">,
  titleWidth: number,
): WaitingRoomRow[] {
  const slices = waitingRoomAllSlices(remote)
  const inventoryLoading = remote.inventoryStatus === "loading"
  const rows: WaitingRoomRow[] = [{
    id: "slices-header",
    title: "Slices",
    value: inventoryLoading && slices.length === 0
      ? waitingRoomLoadingText(remote.loadingFrame)
      : `${slices.length} configured`,
    titleWidth,
    indent: 0,
    focused: false,
    selectable: false,
    scrollbar: "",
  }]

  if (slices.length === 0) {
    rows.push({
      id: "slices-none",
      title: "Slice Inventory",
      value: inventoryLoading ? waitingRoomLoadingText(remote.loadingFrame) : "none",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  for (const [index, slice] of slices.entries()) {
    rows.push({
      id: `slice:${slice.id}`,
      title: formatWaitingRoomSliceLabel(slice),
      value: formatSliceStatus(slice),
      titleWidth,
      indent: 1,
      focused: state.focus === "slice-entry" && (state.sliceIndex ?? 0) === index,
      selectable: true,
      scrollbar: "",
    })
  }

  return rows
}

export function waitingRoomAllSlices(remote: Pick<WaitingRoomRemoteState, "slices">): SliceRecord[] {
  return (remote.slices ?? [])
    .slice()
    .sort((left, right) => formatWaitingRoomSliceLabel(left).localeCompare(formatWaitingRoomSliceLabel(right)))
}

export function waitingRoomSliceTitleWidth(remote: Pick<WaitingRoomRemoteState, "slices">): number {
  return Math.max(
    "Slice Inventory".length,
    ...waitingRoomAllSlices(remote).map((slice) => formatWaitingRoomSliceLabel(slice).length),
  )
}

function formatSliceStatus(slice: SliceRecord): string {
  const agents = slice.agent_ids?.length ?? 0
  const auth = (slice.provider_auth ?? [])
    .map((entry) => formatSliceProviderAuth(entry, {
      separator: " ",
      includeOrgPlan: false,
    }))
    .filter(Boolean)
    .join(",")
  const worktree = formatSliceScope(slice)
  const relay = formatSliceRelayLabel(slice)
  return [
    slice.status,
    slice.display_mode ?? "headless",
    `${agents} agent${agents === 1 ? "" : "s"}`,
    relay ? `relay ${relay}` : "",
    worktree,
    auth ? `auth ${auth}` : "",
    slice.last_operation_status === "failed"
      ? `last error ${slice.last_error ?? slice.last_operation ?? "failed"}`
      : "",
  ].filter(Boolean).join(" ")
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}

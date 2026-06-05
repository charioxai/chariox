import type { SliceRecord } from "./cli-types.js"
import {
  formatSliceProviderAuth,
  formatSliceProviderList,
  formatSliceRelayLabel,
  formatSliceScope,
  sliceProviderAuthCoverage,
} from "./slice-format.js"
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
  const agents = formatSliceAgentOccupancy(slice.agent_ids ?? [])
  const auth = waitingRoomSliceAuthDetail(slice)
  const worktree = formatSliceScope(slice)
  const relay = formatSliceRelayLabel(slice)
  return [
    slice.status,
    slice.display_mode ?? "headless",
    agents,
    relay ? `relay ${relay}` : "",
    worktree,
    auth ? `auth ${auth}` : "",
    slice.last_operation_status === "failed"
      ? `last error ${slice.last_error ?? slice.last_operation ?? "failed"}`
      : "",
  ].filter(Boolean).join(" ")
}

function formatSliceAgentOccupancy(agentIds: readonly string[]): string {
  const agents = agentIds.map((agent) => agent.trim()).filter(Boolean)
  if (agents.length === 0) {
    return "0 agents"
  }
  const shown = agents.slice(0, 3).join(", ")
  const more = agents.length > 3 ? ` +${agents.length - 3} more` : ""
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${shown}${more}`
}

function waitingRoomSliceAuthDetail(slice: SliceRecord): string {
  const authEntries = slice.provider_auth ?? []
  if (authEntries.length === 0) {
    const coverage = sliceProviderAuthCoverage(slice)
    return coverage.providers.length > 0
      ? `missing ${formatSliceProviderList(coverage.providers)}`
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
  return details.join(",")
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}

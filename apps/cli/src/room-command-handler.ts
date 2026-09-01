import {
  cancelRoomEnvironmentActionRequest,
  getRoomEnvironmentSliceRequest,
  getRoomEnvironmentStateRequest,
  listRoomEnvironmentActionHistoryRequest,
  releaseRoomEnvironmentInputRequest,
  requestRoomEnvironmentInputTakeoverRequest,
  retryRoomEnvironmentRequest,
  saveSliceStateRequest,
  startRoomEnvironmentRequest,
  stopRoomEnvironmentRequest,
  type RoomEnvironmentViewportRequest,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentAction,
  RoomEnvironmentActionCancellationOutcome,
  RoomEnvironmentActionCancellationUpdatedResponse,
  RoomEnvironmentActionHistoryResponse,
  RoomEnvironmentInputReleasedResponse,
  RoomEnvironmentInputTarget,
  RoomEnvironmentSliceResponse,
  RoomEnvironmentSnapshot,
  RoomEnvironmentStateResponse,
  RoomEnvironmentTakeoverUpdatedResponse,
  RoomEnvironmentUpdatedResponse,
  SliceRecord,
  SliceSavedStateRecord,
} from "@chariox/kernel-client/kernel-types"

import type { ParsedSlashCommand } from "./commands.js"
import type { DownloadedRoomEnvironmentScreenshot } from "./room-screenshot-api.js"
import { formatSliceStateSaved } from "./slice-command-handlers.js"

type RoomCommand = Extract<ParsedSlashCommand, { kind: "room" }>
type SliceStateSavedResponse = {
  SliceStateSaved: {
    slice: SliceRecord
    state: SliceSavedStateRecord
  }
}

export type RoomViewerTarget = {
  sessionId: string
  agentId: string
  sliceId: string
}

export type RoomViewerOpenResult = {
  url: string
  opened: boolean
}

export type RoomCommandHandlerDeps = {
  isAttached: () => boolean
  sessionId: () => string
  focusedAgentId?: () => string | null
  send: <TResponse>(request: unknown) => Promise<TResponse>
  reconnectEventStream?: () => Promise<boolean>
  openViewer?: (target: RoomViewerTarget) => Promise<RoomViewerOpenResult | null>
  captureScreenshot?: () => Promise<DownloadedRoomEnvironmentScreenshot>
  appendNotice: (notice: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleRoomSlashCommand(
  deps: RoomCommandHandlerDeps,
  command: RoomCommand,
): Promise<void> {
  const [subcommand] = command.args
  if (subcommand && !["status", "show", "actions", "start", "stop", "retry", "reconnect", "view", "screenshot", "takeover", "release", "cancel", "save"].includes(subcommand)) {
    deps.flashFooter(roomCommandUsage(), "error")
    return
  }
  let startViewport: RoomEnvironmentViewportRequest | undefined
  let inputTarget: RoomEnvironmentInputTarget | undefined
  let actionId: string | undefined
  let actionHistory: { limit: number; beforeSequence: number | null } | undefined
  let saveMode: "restart_agents" | "shutdown" | undefined
  if (subcommand === "start") {
    const parsedViewport = parseStartViewport(command.args.slice(1))
    if (typeof parsedViewport === "string") {
      deps.flashFooter(parsedViewport, "error")
      return
    }
    startViewport = parsedViewport
  } else if (subcommand === "takeover" || subcommand === "release") {
    const parsedTarget = parseInputTarget(command.args.slice(1))
    if (typeof parsedTarget === "string") {
      deps.flashFooter(parsedTarget, "error")
      return
    }
    inputTarget = parsedTarget
  } else if (subcommand === "cancel") {
    if (command.args.length !== 2 || !command.args[1]) {
      deps.flashFooter(roomCancelUsage(), "error")
      return
    }
    actionId = command.args[1]
  } else if (subcommand === "actions") {
    const parsedHistory = parseActionHistoryArgs(command.args.slice(1))
    if (typeof parsedHistory === "string") {
      deps.flashFooter(parsedHistory, "error")
      return
    }
    actionHistory = parsedHistory
  } else if (subcommand === "save") {
    if (command.args.length !== 2 || !command.args[1] || !["restart", "shutdown"].includes(command.args[1])) {
      deps.flashFooter(roomSaveUsage(), "error")
      return
    }
    saveMode = command.args[1] === "restart" ? "restart_agents" : "shutdown"
  } else if (command.args.length > 1) {
    deps.flashFooter(roomCommandUsage(), "error")
    return
  }
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a Room to inspect its environment", "error")
    return
  }
  const sessionId = deps.sessionId()
  if (subcommand === "reconnect") {
    if (!deps.reconnectEventStream) {
      deps.flashFooter("Room reconnect is unavailable in this client", "error")
      return
    }
    if (!await deps.reconnectEventStream()) {
      deps.flashFooter("Room reconnect is unavailable on this polling transport", "error")
      return
    }
    deps.appendNotice("Room reconnect requested; events will resume from the last received event.")
    return
  }
  if (subcommand === "screenshot") {
    if (!deps.captureScreenshot) {
      deps.flashFooter("Room Environment screenshot capture is unavailable in this client", "error")
      return
    }
    const screenshot = await deps.captureScreenshot()
    deps.appendNotice([
      "Room Environment screenshot saved.",
      `path=${screenshot.path}`,
      `artifact=${screenshot.artifact.artifact_id}`,
      `sha256=${screenshot.artifact.sha256}`,
    ].join("\n"))
    return
  }
  if (subcommand === "view") {
    const agentId = deps.focusedAgentId?.()
    if (!agentId) {
      deps.flashFooter("focus an agent before opening the Room Environment", "error")
      return
    }
    const bindingResponse = await deps.send<RoomEnvironmentSliceResponse>(
      getRoomEnvironmentSliceRequest(sessionId),
    )
    if (!bindingResponse || typeof bindingResponse !== "object" || !("RoomEnvironmentSlice" in bindingResponse)) {
      throw new Error("Room Environment slice response is malformed")
    }
    const binding = bindingResponse.RoomEnvironmentSlice.binding
    if (!binding) {
      deps.flashFooter("Room Environment has no bound slice to view", "error")
      return
    }
    const opened = await deps.openViewer?.({
      sessionId,
      agentId,
      sliceId: binding.slice_id,
    })
    if (!opened) {
      deps.flashFooter("Chariox Cloud Web View is not configured; run /cloud link first", "error")
      return
    }
    deps.appendNotice([
      "Opening Room Environment in Chariox Cloud.",
      `url=${opened.url}`,
      opened.opened ? "browser=opened" : "browser=manual",
    ].join("\n"))
    return
  }
  if (!subcommand || subcommand === "status" || subcommand === "show") {
    const response = await deps.send<RoomEnvironmentStateResponse>(
      getRoomEnvironmentStateRequest(sessionId),
    )
    if (!response || typeof response !== "object" || !("RoomEnvironmentState" in response)) {
      throw new Error("Room Environment state response is malformed")
    }
    deps.appendNotice(formatRoomEnvironmentStatus(response.RoomEnvironmentState.environment))
    return
  }
  if (subcommand === "actions") {
    if (!actionHistory) throw new Error("Room Environment action history arguments are missing")
    const response = await deps.send<RoomEnvironmentActionHistoryResponse>(
      listRoomEnvironmentActionHistoryRequest(
        sessionId,
        actionHistory.beforeSequence,
        actionHistory.limit,
      ),
    )
    if (!response || typeof response !== "object" || !("RoomEnvironmentActionHistoryListed" in response)) {
      throw new Error("Room Environment action history response is malformed")
    }
    deps.appendNotice(formatRoomEnvironmentActionHistory(response.RoomEnvironmentActionHistoryListed.page))
    return
  }

  if (subcommand === "takeover") {
    if (!inputTarget) throw new Error("Room Environment takeover target is missing")
    const response = await deps.send<RoomEnvironmentTakeoverUpdatedResponse>(
      requestRoomEnvironmentInputTakeoverRequest(sessionId, inputTarget),
    )
    if (!response || typeof response !== "object" || !("RoomEnvironmentTakeoverUpdated" in response)) {
      throw new Error("Room Environment takeover response is malformed")
    }
    const { environment, outcome } = response.RoomEnvironmentTakeoverUpdated
    const result = outcome.state === "granted"
      ? "Room takeover granted"
      : `Room takeover requires cancellation: ${outcome.action_ids.join(", ") || "pending actions"}`
    deps.appendNotice(`${result}\n${formatRoomEnvironmentStatus(environment)}`)
    return
  }
  if (subcommand === "release") {
    if (!inputTarget) throw new Error("Room Environment release target is missing")
    const response = await deps.send<RoomEnvironmentInputReleasedResponse>(
      releaseRoomEnvironmentInputRequest(sessionId, inputTarget),
    )
    if (!response || typeof response !== "object" || !("RoomEnvironmentInputReleased" in response)) {
      throw new Error("Room Environment input release response is malformed")
    }
    deps.appendNotice(`Room input released\n${formatRoomEnvironmentStatus(response.RoomEnvironmentInputReleased.environment)}`)
    return
  }
  if (subcommand === "cancel") {
    if (!actionId) throw new Error("Room Environment action ID is missing")
    const response = await deps.send<RoomEnvironmentActionCancellationUpdatedResponse>(
      cancelRoomEnvironmentActionRequest(sessionId, actionId),
    )
    if (!response || typeof response !== "object" || !("RoomEnvironmentActionCancellationUpdated" in response)) {
      throw new Error("Room Environment action cancellation response is malformed")
    }
    const { environment, outcome } = response.RoomEnvironmentActionCancellationUpdated
    deps.appendNotice(`Room action ${actionId} ${formatCancellationOutcome(outcome)}\n${formatRoomEnvironmentStatus(environment)}`)
    return
  }
  if (subcommand === "save") {
    if (!saveMode) throw new Error("Room Environment save mode is missing")
    const bindingResponse = await deps.send<RoomEnvironmentSliceResponse>(
      getRoomEnvironmentSliceRequest(sessionId),
    )
    if (!bindingResponse || typeof bindingResponse !== "object" || !("RoomEnvironmentSlice" in bindingResponse)) {
      throw new Error("Room Environment slice response is malformed")
    }
    const binding = bindingResponse.RoomEnvironmentSlice.binding
    if (!binding) {
      deps.flashFooter("Room Environment has no bound slice to save", "error")
      return
    }
    const response = await deps.send<SliceStateSavedResponse>(
      saveSliceStateRequest(binding.slice_id, saveMode, "this_slice"),
    )
    if (!response || typeof response !== "object" || !("SliceStateSaved" in response)) {
      throw new Error("Room Environment slice save response is malformed")
    }
    deps.appendNotice(formatSliceStateSaved(response.SliceStateSaved.slice, response.SliceStateSaved.state))
    return
  }

  let request: unknown
  if (subcommand === "start") {
    if (!startViewport) throw new Error("Room Environment start viewport is missing")
    request = startRoomEnvironmentRequest(sessionId, startViewport)
  } else if (subcommand === "stop") {
    request = stopRoomEnvironmentRequest(sessionId)
  } else {
    request = retryRoomEnvironmentRequest(sessionId)
  }
  const response = await deps.send<RoomEnvironmentUpdatedResponse>(request)
  if (!response || typeof response !== "object" || !("RoomEnvironmentUpdated" in response)) {
    throw new Error("Room Environment lifecycle response is malformed")
  }
  deps.appendNotice(formatRoomEnvironmentStatus(response.RoomEnvironmentUpdated.environment))
}

function parseInputTarget(args: string[]): RoomEnvironmentInputTarget | string {
  if (args.length === 0 || (args.length === 1 && args[0] === "desktop")) {
    return { kind: "desktop" }
  }
  if (args.length === 2 && args[0] === "tab" && args[1]) {
    return { kind: "browser_tab", id: args[1] }
  }
  return roomInputUsage()
}

function parseActionHistoryArgs(
  args: string[],
): { limit: number; beforeSequence: number | null } | string {
  if (args.length > 2) return roomActionsUsage()
  const limit = Number(args[0] ?? "20")
  const beforeSequence = args[1] == null ? null : Number(args[1])
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 100) return roomActionsUsage()
  if (beforeSequence != null && (!Number.isSafeInteger(beforeSequence) || beforeSequence < 1)) {
    return roomActionsUsage()
  }
  return { limit, beforeSequence }
}

function parseStartViewport(args: string[]): RoomEnvironmentViewportRequest | string {
  if (args.length > 2) return roomStartUsage()
  const dimensions = args[0] ?? "1280x800"
  const match = /^(\d+)x(\d+)$/i.exec(dimensions)
  const scale = Number(args[1] ?? "1")
  if (!match || !isU32(scale)) return roomStartUsage()
  const cssWidth = Number(match[1])
  const cssHeight = Number(match[2])
  if (!isU32(cssWidth) || !isU32(cssHeight)) return roomStartUsage()
  const desktopPixelWidth = cssWidth * scale
  const desktopPixelHeight = cssHeight * scale
  if (!isU32(desktopPixelWidth) || !isU32(desktopPixelHeight)) return roomStartUsage()
  return {
    css_width: cssWidth,
    css_height: cssHeight,
    device_scale_factor: scale,
    desktop_pixel_width: desktopPixelWidth,
    desktop_pixel_height: desktopPixelHeight,
  }
}

function isU32(value: number): boolean {
  return Number.isSafeInteger(value) && value > 0 && value <= 0xffff_ffff
}

function roomCommandUsage(): string {
  return "usage: /room status|actions [LIMIT] [BEFORE_SEQUENCE]|start [WIDTHxHEIGHT] [SCALE]|stop|retry|reconnect|view|screenshot|takeover|release [desktop|tab TAB_ID]|cancel ACTION_ID|save restart|shutdown"
}

function roomActionsUsage(): string {
  return "usage: /room actions [LIMIT] [BEFORE_SEQUENCE]"
}

function roomStartUsage(): string {
  return "usage: /room start [WIDTHxHEIGHT] [SCALE]"
}

function roomInputUsage(): string {
  return "usage: /room takeover|release [desktop|tab TAB_ID]"
}

function roomCancelUsage(): string {
  return "usage: /room cancel ACTION_ID"
}

function roomSaveUsage(): string {
  return "usage: /room save restart|shutdown"
}

function formatCancellationOutcome(outcome: RoomEnvironmentActionCancellationOutcome): string {
  switch (outcome.state) {
    case "cancelled":
      return "cancelled"
    case "cancellation_requested":
      return "cancellation requested"
    case "already_terminal":
      return `already ${outcome.action_state}`
  }
}

export function formatRoomEnvironmentStatus(environment: RoomEnvironmentSnapshot): string {
  const viewport = environment.viewport
  const focusedTab = environment.tabs.find((tab) => tab.tab_id === environment.focused_tab_id)
    ?? environment.tabs.find((tab) => tab.focused)
  const actors = environment.actors
    .map((actor) => `${actor.display_label} (${actor.kind},${actor.presence})`)
    .join(", ") || "none"
  const input = environment.input_ownership
    .map((owner) => {
      const actor = environment.actors.find((candidate) => candidate.actor_id === owner.actor_id)
      const target = owner.target.kind === "desktop" ? "desktop" : `tab:${owner.target.id}`
      return `${target}:${actor?.display_label ?? actorLabel(owner.actor_id)}`
    })
    .join(", ") || "available"
  return [
    `Room environment ${environment.environment_id}`,
    `lifecycle=${environment.lifecycle} generation=${environment.runtime_generation} cursor=${environment.event_cursor}`,
    `health=${environment.health.map(formatHealth).join(", ") || "none"}`,
    `viewport=${viewport.desktop_pixel_width}x${viewport.desktop_pixel_height} css=${viewport.css_width}x${viewport.css_height} scale=${viewport.device_scale_factor} revision=${viewport.revision}`,
    `tab=${focusedTab ? `${focusedTab.tab_id} ${tabLabel(focusedTab.title, focusedTab.url)}` : "none"}`,
    `actors=${actors}`,
    `input=${input}`,
    `last_action=${formatLastAction(environment)}`,
  ].join("\n")
}

function formatRoomEnvironmentActionHistory(
  page: RoomEnvironmentActionHistoryResponse["RoomEnvironmentActionHistoryListed"]["page"],
): string {
  if (page.actions.length === 0) return "Room actions: none"
  const lines = page.actions.map((action) => {
    const targets = action.targets.map((target) => (
      target.kind === "desktop" ? "desktop" : `tab:${target.id}`
    )).join(",") || "none"
    return [
      `#${action.sequence}`,
      action.action_id,
      `actor=${action.actor_id}`,
      `${action.mode}:${action.kind}`,
      `target=${targets}`,
      `state=${formatHistoryActionState(action)}`,
      `submitted_at_ms=${action.submitted_at_ms}`,
    ].join(" ")
  })
  return [
    `Room actions (${page.actions.length})`,
    ...lines,
    `next_before=${page.next_before_sequence ?? "none"}`,
  ].join("\n")
}

function formatHistoryActionState(action: RoomEnvironmentAction): string {
  if (action.outcome?.status === "failed") return `failed(${action.outcome.code})`
  if (action.outcome?.status === "cancelled") return `cancelled(${action.outcome.reason})`
  return action.state
}

function formatHealth(health: RoomEnvironmentSnapshot["health"][number]): string {
  const diagnostic = health.diagnostic_code ? `(${health.diagnostic_code})` : ""
  return `${health.component}:${health.state}${diagnostic}`
}

function tabLabel(title: string, url: string): string {
  const normalizedTitle = title.trim()
  return normalizedTitle && normalizedTitle !== url ? `${normalizedTitle} — ${url}` : url
}

function formatLastAction(environment: RoomEnvironmentSnapshot): string {
  const action = environment.actions.reduce<RoomEnvironmentAction | null>(
    (latest, candidate) => !latest || candidate.sequence > latest.sequence ? candidate : latest,
    null,
  )
  if (!action) return "none"
  const actor = environment.actors.find((candidate) => candidate.actor_id === action.actor_id)
  return `${action.action_id} ${action.mode}:${action.kind} ${action.state} actor=${actor?.display_label ?? actorLabel(action.actor_id)}`
}

function actorLabel(actorId: string): string {
  const separator = actorId.indexOf(":")
  return separator >= 0 ? actorId.slice(separator + 1) : actorId
}

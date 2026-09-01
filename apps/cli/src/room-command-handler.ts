import {
  getRoomEnvironmentStateRequest,
  retryRoomEnvironmentRequest,
  startRoomEnvironmentRequest,
  stopRoomEnvironmentRequest,
  type RoomEnvironmentViewportRequest,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentAction,
  RoomEnvironmentSnapshot,
  RoomEnvironmentStateResponse,
  RoomEnvironmentUpdatedResponse,
} from "@chariox/kernel-client/kernel-types"

import type { ParsedSlashCommand } from "./commands.js"

type RoomCommand = Extract<ParsedSlashCommand, { kind: "room" }>

export type RoomCommandHandlerDeps = {
  isAttached: () => boolean
  sessionId: () => string
  send: <TResponse>(request: unknown) => Promise<TResponse>
  appendNotice: (notice: string) => void
  flashFooter: (message: string, tone: "info" | "error") => void
}

export async function handleRoomSlashCommand(
  deps: RoomCommandHandlerDeps,
  command: RoomCommand,
): Promise<void> {
  const [subcommand] = command.args
  if (subcommand && !["status", "show", "start", "stop", "retry"].includes(subcommand)) {
    deps.flashFooter(roomCommandUsage(), "error")
    return
  }
  let startViewport: RoomEnvironmentViewportRequest | undefined
  if (subcommand === "start") {
    const parsedViewport = parseStartViewport(command.args.slice(1))
    if (typeof parsedViewport === "string") {
      deps.flashFooter(parsedViewport, "error")
      return
    }
    startViewport = parsedViewport
  } else if (command.args.length > 1) {
    deps.flashFooter(roomCommandUsage(), "error")
    return
  }
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a Room to inspect its environment", "error")
    return
  }
  const sessionId = deps.sessionId()
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
  return "usage: /room status|start [WIDTHxHEIGHT] [SCALE]|stop|retry"
}

function roomStartUsage(): string {
  return "usage: /room start [WIDTHxHEIGHT] [SCALE]"
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

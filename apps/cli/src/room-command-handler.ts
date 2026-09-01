import {
  getRoomEnvironmentStateRequest,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentAction,
  RoomEnvironmentSnapshot,
  RoomEnvironmentStateResponse,
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
  if (subcommand && subcommand !== "status" && subcommand !== "show") {
    deps.flashFooter("usage: /room status", "error")
    return
  }
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a Room to inspect its environment", "error")
    return
  }
  const response = await deps.send<RoomEnvironmentStateResponse>(
    getRoomEnvironmentStateRequest(deps.sessionId()),
  )
  if (!response || typeof response !== "object" || !("RoomEnvironmentState" in response)) {
    throw new Error("Room Environment state response is malformed")
  }
  deps.appendNotice(formatRoomEnvironmentStatus(response.RoomEnvironmentState.environment))
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

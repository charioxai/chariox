import { waitingRoomSessionRecencyMs } from "./waiting-room-activity.js"

export type SessionBrowserKeyEvent = {
  name: string
  eventType?: string | undefined
  ctrl?: boolean | undefined
  meta?: boolean | undefined
  alt?: boolean | undefined
  super?: boolean | undefined
}

export type SessionBrowserKeyAction =
  | { action: "ignore" }
  | { action: "close" }
  | { action: "move"; delta: number }
  | { action: "empty" }
  | { action: "submit"; selectedIndex: number }
  | { action: "lifecycle"; selectedIndex: number; lifecycleAction: "archive" | "delete" }
  | { action: "handled" }

export type SessionBrowserSession = {
  status: string
  last_prompt_sent_at_ms?: number | null
  last_activity_at_ms?: number | null
  last_used_at_ms?: number | null
  created_at_ms?: number | null
}

export function resolveSessionBrowserKeyAction(options: {
  open: boolean
  event: SessionBrowserKeyEvent
  sessionCount: number
  selectedIndex: number
}): SessionBrowserKeyAction {
  const event = options.event
  if (
    !options.open
    || event.eventType === "release"
    || event.ctrl
    || event.meta
    || event.alt
    || event.super
  ) {
    return { action: "ignore" }
  }
  if (event.name === "escape") {
    return { action: "close" }
  }
  if (event.name === "up") {
    return { action: "move", delta: -1 }
  }
  if (event.name === "down") {
    return { action: "move", delta: 1 }
  }
  if (options.sessionCount <= 0 || options.selectedIndex < 0 || options.selectedIndex >= options.sessionCount) {
    return { action: "empty" }
  }
  if (event.name === "return" || event.name === "enter") {
    return { action: "submit", selectedIndex: options.selectedIndex }
  }
  if (event.name === "a") {
    return { action: "lifecycle", selectedIndex: options.selectedIndex, lifecycleAction: "archive" }
  }
  if (event.name === "d" || event.name === "delete") {
    return { action: "lifecycle", selectedIndex: options.selectedIndex, lifecycleAction: "delete" }
  }
  return { action: "handled" }
}

export function nextSessionBrowserIndex(index: number, delta: number, sessionCount: number): number {
  if (sessionCount <= 0) {
    return index
  }
  const next = index + delta
  return ((next % sessionCount) + sessionCount) % sessionCount
}

export function sessionBrowserVisibleSessions<T extends SessionBrowserSession>(sessions: readonly T[]): T[] {
  return sessions
    .filter((session) => session.status !== "Ended")
    .slice()
    .sort((left, right) => waitingRoomSessionRecencyMs(right) - waitingRoomSessionRecencyMs(left))
}

export function clampSessionBrowserIndex(index: number, sessionCount: number): number {
  return Math.min(Math.max(0, index), Math.max(0, sessionCount - 1))
}

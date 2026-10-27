import { keybindFromEvent, matchKeybind, parseKeybinds, type ParsedShortcut } from "./keybind.js"

export type ShortcutEvent = ParsedShortcut

export type TabFocusCycleContext = {
  attached: boolean
  hotkeysOpen: boolean
  promptFocused: boolean
  commandCenterOpen: boolean
  commandCenterQuery: string
}

export type WaitingRoomKeyContext = TabFocusCycleContext

export type HotkeysToggleMatch = {
  matched: boolean
  normalizedName: string
  reason: "release" | "ctrl+t" | "meta+t" | "super+t" | "non_toggle_key" | "missing_modifier" | "unsupported_modifier"
}

export const HOTKEY_TOGGLE_LABEL = "Ctrl+T"
const CTRL_T = parseKeybinds("ctrl+t")
const META_T = parseKeybinds("meta+t")
const SUPER_T = parseKeybinds("super+t")

export function matchHotkeysToggleEvent(event: ShortcutEvent, platform = process.platform): HotkeysToggleMatch {
  const parsed = keybindFromEvent(event)
  const name = parsed.name
  if (event.eventType === "release") {
    return { matched: false, normalizedName: name, reason: "release" }
  }

  if (name !== "t") {
    return { matched: false, normalizedName: name, reason: "non_toggle_key" }
  }

  if (CTRL_T.some((binding) => matchKeybind(binding, parsed))) {
    return { matched: true, normalizedName: name, reason: "ctrl+t" }
  }

  if (platform === "darwin" && META_T.some((binding) => matchKeybind(binding, parsed))) {
    return { matched: true, normalizedName: name, reason: "meta+t" }
  }

  if (platform === "darwin" && SUPER_T.some((binding) => matchKeybind(binding, parsed))) {
    return { matched: true, normalizedName: name, reason: "super+t" }
  }

  if (event.meta || event.super) {
    return { matched: false, normalizedName: name, reason: "unsupported_modifier" }
  }

  return { matched: false, normalizedName: name, reason: "missing_modifier" }
}

export function isHotkeysToggleEvent(event: ShortcutEvent, platform = process.platform) {
  return matchHotkeysToggleEvent(event, platform).matched
}

export function shouldCycleFocusOnTabEvent(
  event: ShortcutEvent,
  context: TabFocusCycleContext,
) {
  if (event.eventType === "release" || event.name !== "tab") {
    return false
  }
  if (!context.attached || context.hotkeysOpen) {
    return false
  }
  if (context.promptFocused && (context.commandCenterOpen || context.commandCenterQuery.startsWith("/"))) {
    return false
  }
  return true
}

export function shouldHandleWaitingRoomKeyEvent(
  event: ShortcutEvent,
  context: WaitingRoomKeyContext,
) {
  if (context.attached || context.hotkeysOpen) {
    return false
  }
  if (context.commandCenterOpen) {
    return false
  }
  if (context.promptFocused && context.commandCenterQuery.trim().length > 0) {
    return false
  }
  return true
}

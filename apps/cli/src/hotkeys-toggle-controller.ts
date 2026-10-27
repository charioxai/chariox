import {
  matchHotkeysToggleEvent,
  type HotkeysToggleMatch,
  type ShortcutEvent,
} from "./hotkeys.js"

export type HotkeysToggleSource = "keyboard" | "stdin" | "textarea"

export type HotkeysToggleEvent = ShortcutEvent & {
  defaultPrevented?: boolean
  preventDefault?: () => void
  stopPropagation?: () => void
}

export type HotkeysToggleControllerDeps = {
  hotkeysOpen: () => boolean
  toggleHotkeys: () => void
  debugHotkey: (message: string) => void
  logDebug: (message: string, fields: Record<string, unknown>) => void
  currentFocus: () => unknown
  describeFocus: (focus: unknown) => unknown
  savedFocusDebug: () => { type?: string | null } | null | undefined
  matchEvent?: (event: ShortcutEvent) => HotkeysToggleMatch
}

export function createHotkeysToggleController(
  deps: HotkeysToggleControllerDeps,
) {
  const matchEvent = deps.matchEvent ?? matchHotkeysToggleEvent

  const inspect = (
    source: HotkeysToggleSource,
    event: ShortcutEvent,
  ) => {
    const match = matchEvent(event)
    if (match.normalizedName === "t" || event.ctrl || event.meta || event.super) {
      deps.logDebug("evaluated hotkeys toggle shortcut", {
        source,
        matched: match.matched,
        reason: match.reason,
        key_name: event.name,
        normalized_name: match.normalizedName,
        event_type: event.eventType ?? null,
        ctrl: Boolean(event.ctrl),
        meta: Boolean(event.meta),
        super: Boolean(event.super),
        base_code: event.baseCode ?? null,
        hotkeys_open: deps.hotkeysOpen(),
      })
    }
    return match
  }

  const handle = (
    source: HotkeysToggleSource,
    event: HotkeysToggleEvent,
  ) => {
    if (event.defaultPrevented) {
      return false
    }
    const hotkeysToggle = inspect(source, event)
    if (!hotkeysToggle.matched) {
      return false
    }

    event.preventDefault?.()
    event.stopPropagation?.()
    const previousHotkeysOpen = deps.hotkeysOpen()
    deps.debugHotkey(`shortcut ${source} matched reason=${hotkeysToggle.reason} open=${previousHotkeysOpen} key=${event.name}`)
    deps.logDebug("toggling hotkeys via shortcut", {
      source,
      reason: hotkeysToggle.reason,
      hotkeys_open: previousHotkeysOpen,
      next_hotkeys_open: !previousHotkeysOpen,
      current_focus: deps.describeFocus(deps.currentFocus()),
    })
    deps.toggleHotkeys()
    deps.debugHotkey(`shortcut ${source} finished open=${deps.hotkeysOpen()} saved=${deps.savedFocusDebug()?.type ?? "none"}`)
    deps.logDebug("finished toggling hotkeys via shortcut", {
      source,
      reason: hotkeysToggle.reason,
      previous_hotkeys_open: previousHotkeysOpen,
      hotkeys_open: deps.hotkeysOpen(),
      saved_focus: deps.savedFocusDebug(),
      current_focus: deps.describeFocus(deps.currentFocus()),
    })
    return true
  }

  return {
    inspect,
    handle,
  }
}

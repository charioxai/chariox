export type ParsedShortcut = {
  name: string
  ctrl?: boolean
  meta?: boolean
  shift?: boolean
  super?: boolean
  eventType?: string
  baseCode?: number
}

export type KeybindInfo = Pick<ParsedShortcut, "name" | "ctrl" | "meta" | "shift" | "super"> & {
  leader: boolean
}

export function keybindFromEvent(event: ParsedShortcut, leader = false): KeybindInfo {
  return {
    name: normalizeKeybindName(event),
    ctrl: Boolean(event.ctrl),
    meta: Boolean(event.meta),
    shift: Boolean(event.shift),
    super: Boolean(event.super),
    leader,
  }
}

export function matchKeybind(binding: KeybindInfo | undefined, event: KeybindInfo) {
  if (!binding) {
    return false
  }
  return binding.name === event.name
    && Boolean(binding.ctrl) === Boolean(event.ctrl)
    && Boolean(binding.meta) === Boolean(event.meta)
    && Boolean(binding.shift) === Boolean(event.shift)
    && Boolean(binding.super) === Boolean(event.super)
    && Boolean(binding.leader) === Boolean(event.leader)
}

export function parseKeybinds(value: string) {
  if (!value.trim()) {
    return []
  }
  return value.split(",").map((combo) => {
    const normalized = combo.replace(/<leader>/gi, "leader+")
    const parts = normalized.toLowerCase().split("+")
    const info: KeybindInfo = {
      name: "",
      ctrl: false,
      meta: false,
      shift: false,
      super: false,
      leader: false,
    }
    for (const rawPart of parts) {
      const part = rawPart.trim()
      switch (part) {
        case "ctrl":
          info.ctrl = true
          break
        case "alt":
        case "meta":
        case "option":
          info.meta = true
          break
        case "super":
        case "cmd":
        case "command":
          info.super = true
          break
        case "shift":
          info.shift = true
          break
        case "leader":
          info.leader = true
          break
        case "esc":
          info.name = "escape"
          break
        case "space":
          info.name = "space"
          break
        default:
          info.name = part
          break
      }
    }
    return info
  })
}

function normalizeKeybindName(event: ParsedShortcut) {
  if (typeof event.baseCode === "number" && event.baseCode >= 0) {
    const base = String.fromCodePoint(event.baseCode)
    if (base.length === 1) {
      return base === " " ? "space" : base.toLowerCase()
    }
  }
  if (event.name === " ") {
    return "space"
  }
  return event.name.toLowerCase()
}

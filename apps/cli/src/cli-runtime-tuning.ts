import type { KeyBinding } from "@opentui/core"

export const PROMPT_KEYBINDINGS = [
  { name: "return", action: "submit" },
  { name: "return", shift: true, action: "newline" },
  { name: "return", meta: true, action: "newline" },
] satisfies KeyBinding[]

export const LIVE_TRANSCRIPT_LIMIT = 400
export const LIVE_TRANSCRIPT_MAX_CHARS = 250_000
export const STREAM_BATCH_WINDOW_MS = 48
export const CHROME_UPDATE_THROTTLE_MS = 48
export const TURN_COMPLETION_QUIET_MS = 1_500
export const COMMAND_CENTER_OVERLAY_FOOTPRINT = 3
export const ATTACHED_PROMPT_PLACEHOLDER = "Write your next prompt here"

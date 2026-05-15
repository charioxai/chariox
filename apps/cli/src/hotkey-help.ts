import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"

export type HotkeyItem = {
  keys: string
  description: string
}

export type HotkeySection = {
  title: string
  items: HotkeyItem[]
}

const GLOBAL_HOTKEYS: HotkeyItem[] = [
  { keys: HOTKEY_TOGGLE_LABEL, description: "Show or hide this hotkey list." },
  { keys: "Ctrl+E", description: "Exit the CLI with the same behavior as /exit." },
  { keys: "Ctrl+C", description: "Stop the active agent; if idle, exit the CLI." },
]

const SESSION_HOTKEYS: HotkeyItem[] = [
  { keys: "Enter", description: "Submit the current prompt." },
  { keys: "Shift+Enter", description: "Insert a newline in the prompt." },
  { keys: "Tab", description: "Cycle focus to the next agent or workflow node." },
  { keys: "Ctrl+P", description: "Toggle between the agent screens and workflow outline." },
  { keys: "Up / Down", description: "Browse submitted prompts in the prompt area." },
  { keys: "Shift+Up / Shift+Down", description: "Jump between user turns when the prompt is empty." },
  { keys: "Backspace / Delete", description: "Remove pending attachment tokens from the prompt." },
]

const WAITING_ROOM_HOTKEYS: HotkeyItem[] = [
  { keys: "Arrow keys", description: "Move through options and browse the visible session preview." },
  { keys: "Enter", description: "Create, attach, or open the full session list from Join Existing Session." },
  { keys: "A", description: "Archive the selected session, or all sessions from Join Existing Session, after confirmation." },
  { keys: "D / Delete", description: "Delete the selected session, all sessions, or inactive remote inventory after confirmation." },
]

export function buildHotkeySections(attached: boolean): HotkeySection[] {
  return [
    { title: "Global", items: GLOBAL_HOTKEYS },
    attached
      ? { title: "Session", items: SESSION_HOTKEYS }
      : { title: "Waiting room", items: WAITING_ROOM_HOTKEYS },
  ]
}

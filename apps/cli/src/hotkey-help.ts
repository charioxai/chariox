import { approvalShortcutLabel } from "./approval-shortcuts.js"
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
  { keys: "F7 / Esc", description: "F7 turns mouse reporting off: drag-select and press Cmd-C. Esc or F7 restores the mouse." },
  { keys: "F6 / Meta+C", description: "Copy the selected text (drag release also copies). F7 enables native copy over SSH." },
  { keys: "Native selection", description: "Hold Shift while dragging (terminal dependent), then use terminal Copy. Set CHARIOX_TUI_MOUSE=off to disable mouse capture." },
]

const SESSION_HOTKEYS: HotkeyItem[] = [
  { keys: `${approvalShortcutLabel()} · /approvals`, description: "Open pending approvals; critical actions need your passkey." },
  { keys: "Enter", description: "Submit the current prompt." },
  { keys: "Shift+Enter", description: "Insert a newline in the prompt." },
  { keys: "Tab", description: "Cycle focus to the next agent or workflow node." },
  { keys: "Ctrl+P", description: "Toggle between the agent screens and workflow outline." },
  { keys: "Up / Down", description: "Browse submitted prompts in the prompt area." },
  { keys: "Shift+Up / Shift+Down", description: "Jump between user turns when the prompt is empty." },
  { keys: "Backspace / Delete", description: "Remove pending attachment tokens from the prompt." },
]

const WAITING_ROOM_HOTKEYS: HotkeyItem[] = [
  { keys: "Arrow keys", description: "Move through options, projects, sessions, and remote inventory." },
  { keys: "Enter", description: "Create, attach, or open the selected project's sessions." },
  { keys: "E", description: "Rename the selected project." },
  { keys: "A", description: "Archive the selected project or session after confirmation." },
  { keys: "D / Delete", description: "Delete the selected project, session, or inactive remote inventory after confirmation." },
  { keys: "R", description: "Restore the selected archived project." },
]

export function buildHotkeySections(attached: boolean): HotkeySection[] {
  return [
    { title: "Global", items: GLOBAL_HOTKEYS },
    attached
      ? { title: "Session", items: SESSION_HOTKEYS }
      : { title: "Waiting room", items: WAITING_ROOM_HOTKEYS },
  ]
}

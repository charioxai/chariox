import type { ParsedShortcut } from "./keybind.js"

export function approvalShortcutLabel(platform = process.platform): string {
  return `${platform === "darwin" ? "F8 (fn+F8 on Mac)" : "F8"} or Ctrl+G`
}

/** Ctrl+G has no prompt editing or existing Chariox shortcut binding. */
export function isApprovalShortcut(event: ParsedShortcut & { alt?: boolean }): boolean {
  if (event.meta || event.super || event.alt || event.shift) return false
  return (event.name.toLowerCase() === "g" && Boolean(event.ctrl))
    || (event.name === "f8" && !event.ctrl)
}

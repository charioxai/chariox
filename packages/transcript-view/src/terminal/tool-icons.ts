import type { ToolDisplayIcon } from "@chariox/tool-display"

// One outline icon per tool kind (tool-display `ToolDisplay.icon`), drawn
// left of a tool row's title in the pane's text color.
const toolIconPaths: Readonly<Record<ToolDisplayIcon, string>> = {
  terminal: `<path d="M4 17l6-6-6-6"/><path d="M12 19h8"/>`,
  read: `<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6"/><path d="M8 13h8M8 17h6"/>`,
  edit: `<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>`,
  search: `<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/>`,
  web: `<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a14 14 0 0 1 0 18a14 14 0 0 1 0-18"/>`,
  browser: `<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/><path d="M7 6.5h.01M10 6.5h.01"/>`,
  mcp: `<path d="M9 2v6M15 2v6"/><path d="M6 8h12v4a6 6 0 0 1-12 0z"/><path d="M12 18v4"/>`,
  agent: `<rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 8V4"/><path d="M9 14h.01M15 14h.01"/>`,
  plan: `<path d="M3 6l2 2 3-3"/><path d="M3 15l2 2 3-3"/><path d="M11 7h10M11 16h10"/>`,
  tool: `<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-.6-.6-2.4z"/>`,
}

export function isToolDisplayIcon(value: unknown): value is ToolDisplayIcon {
  return typeof value === "string" && Object.hasOwn(toolIconPaths, value)
}

export function renderToolIcon(icon: ToolDisplayIcon): string {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" focusable="false">${toolIconPaths[icon]}</svg>`
}

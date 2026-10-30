import {
  formatToolDisplay,
  type ToolDisplay,
} from "@chariox/tool-display"
import type {
  TerminalOutputEntry,
} from "./runtime-state.js"

export function terminalActivitySummary(text: string): string {
  const singleLine = text
    .replace(/^\s{0,3}#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/\s+/g, " ")
    .trim()
  return singleLine.length > 120 ? `${singleLine.slice(0, 117)}...` : singleLine
}

// Settled turn label counted by item kind, from the shared tool titles:
// "Read 2 files · Ran 3 commands · Thought". Every row counts once.
// One count per tool call: a history row (titled from its preview) and a live
// row of the same call count once.
export function terminalTurnActivitySummary(entries: readonly TerminalOutputEntry[]): string {
  const counts = new Map<string, number>()
  const callIds = new Set<string>()
  for (const entry of entries) {
    const display = entry.role === "tool" ? parseToolDisplay(entry) : null
    const callId = entry.historyBlob?.callId ?? (display ? terminalToolCallIdOf(entry) : null)
    if (callId && callIds.has(callId)) {
      continue
    }
    if (callId) {
      callIds.add(callId)
    }
    const kind = entry.role === "reasoning" ? "thought" : entry.role === "tool" ? toolActivityKind(display?.title ?? entry.historyBlob?.title ?? "") : null
    if (kind) {
      counts.set(kind, (counts.get(kind) ?? 0) + (kind === "edit" ? editedFileCount(entry) : 1))
    }
  }
  return [...counts].map(([kind, count]) => activityKindPhrase(kind, count)).join(" · ")
}

function terminalToolCallIdOf(entry: TerminalOutputEntry): string | null {
  try {
    const id = (JSON.parse(entry.sourceText ?? entry.text) as { id?: unknown }).id
    return typeof id === "string" && id ? id : null
  } catch {
    return null
  }
}

function toolActivityKind(rawTitle: string): string {
  const title = rawTitle.replace(/^.* · /, "")
  const verb = title.split(" ")[0]?.toLowerCase() ?? ""
  if (verb === "ran" || verb === "running") return "command"
  if (verb === "read" || verb === "reading") return title.startsWith("Read plan") || title.startsWith("Reading plan") ? "plan" : "read"
  if (["edited", "editing", "wrote", "writing", "deleted", "deleting", "moved", "moving"].includes(verb)) return "edit"
  if (verb === "searched" || verb === "searching") return "search"
  if (verb === "listed" || verb === "listing") return "list"
  if (verb === "fetched" || verb === "fetching") return "fetch"
  if (verb === "updated" || verb === "updating") return "plan"
  if (verb === "delegated" || verb === "delegating") return "task"
  return "tool"
}

function editedFileCount(entry: TerminalOutputEntry): number {
  const match = parseToolDisplay(entry)?.title.match(/^(?:.* · )?Edit(?:ed|ing) (\d+) files\b/)
  return match ? Number.parseInt(match[1]!, 10) : 1
}

function activityKindPhrase(kind: string, count: number): string {
  switch (kind) {
    case "thought": return count === 1 ? "Thought" : `Thought ×${count}`
    case "command": return `Ran ${countPhrase(count, "command")}`
    case "read": return `Read ${countPhrase(count, "file")}`
    case "edit": return `Edited ${countPhrase(count, "file")}`
    case "search": return `Searched ${count === 1 ? "once" : `${count} times`}`
    case "list": return `Listed ${countPhrase(count, "directory", "directories")}`
    case "fetch": return `Fetched ${countPhrase(count, "page")}`
    case "plan": return "Updated plan"
    case "task": return `Delegated ${countPhrase(count, "task")}`
    default: return `Called ${countPhrase(count, "tool")}`
  }
}

function parseToolDisplay(entry: TerminalOutputEntry): ToolDisplay | null {
  try {
    const parsed = JSON.parse(entry.sourceText ?? entry.text) as Record<string, unknown>
    if (!parsed || (typeof parsed.id !== "string" && typeof parsed.tool !== "string")) {
      return null
    }
    return formatToolDisplay({ ...parsed, id: String(parsed.id ?? "") } as Parameters<typeof formatToolDisplay>[0])
  } catch {
    return null
  }
}

function countPhrase(
  count: number,
  singular: string,
  plural = `${singular}s`,
): string {
  return `${count} ${count === 1 ? singular : plural}`
}

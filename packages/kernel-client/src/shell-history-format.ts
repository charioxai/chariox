import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"

import type {
  HistoryEvent,
  SemanticHistoryMatch,
  SessionHistoryPageEntry,
} from "./kernel-types.js"

export function formatPromptSummary(history: SessionHistoryPageEntry[]): string {
  const summary = history
    .filter((entry) => entry.entry.kind === "provider_output")
    .map((entry) => entry.entry.text.trim())
    .filter(Boolean)
    .join("\n")
  return summary || "(no summary output)"
}

export function formatPromptReply(history: SessionHistoryPageEntry[]): string {
  const tools = new Map<string, ToolTranscriptUpdate>()
  const reply = history
    .map((entry) => formatPromptHistoryEntry(entry, tools))
    .filter(Boolean)
    .join("\n")
  return reply || "(no reply output)"
}

export function formatHistoryEvents(events: HistoryEvent[]): string {
  if (events.length === 0) {
    return "no matching history"
  }
  return events.map((event) => formatHistoryEventLine(event)).join("\n\n")
}

export function formatSemanticHistoryMatches(matches: SemanticHistoryMatch[]): string {
  if (matches.length === 0) {
    return "no semantic matches"
  }
  return matches.map((match) => {
    const score = typeof match.score_millis === "number"
      ? ` score=${(match.score_millis / 1000).toFixed(3)}`
      : ""
    const chunk = typeof match.chunk_index === "number" ? ` chunk=${match.chunk_index}` : ""
    const reason = match.reason ? `\nreason: ${truncateHistoryText(match.reason)}` : ""
    return `${formatHistoryEventLine(match.event)}${score}${chunk}${match.chunk_text ? `\n${truncateHistoryText(match.chunk_text)}` : ""}${reason}`
  }).join("\n\n")
}

export function formatPromptBlob(promptId: string, title: string, content: string): string {
  const indent = "                        "
  const lines = content.split(/\r?\n/)
  return [`${promptId} ${title}`, ...lines.map((line) => `${indent}${line}`)].join("\n")
}

function formatHistoryEventLine(event: HistoryEvent): string {
  const timestamp = Number.isFinite(event.timestamp_ms)
    ? new Date(event.timestamp_ms).toISOString()
    : "unknown-time"
  const label = [
    event.kind,
    event.provider,
    event.model,
    event.session_id ? `session=${event.session_id}` : null,
    event.agent_id ? `agent=${event.agent_id}` : null,
  ].filter(Boolean).join(" ")
  const content = truncateHistoryText(event.content ?? "")
  return content ? `${timestamp} ${label}\n${content}` : `${timestamp} ${label}`
}

function truncateHistoryText(text: string): string {
  const normalized = text.replace(/\s+/g, " ").trim()
  return normalized.length > 320 ? `${normalized.slice(0, 317)}...` : normalized
}

function formatPromptHistoryEntry(
  entry: SessionHistoryPageEntry,
  tools: Map<string, ToolTranscriptUpdate>,
): string {
  const text = entry.entry.text.trim()
  if (!text) return ""
  if (entry.entry.kind === "provider_output") return text
  if (entry.entry.kind !== "provider_tool") return `[${entry.entry.kind}] ${text}`

  const parsed = parseToolTranscriptUpdate(entry.entry.text)
  if (!parsed) return `[${entry.entry.kind}] ${text}`

  const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
  tools.set(parsed.id, merged)
  return formatToolTranscriptUpdate(merged)
}

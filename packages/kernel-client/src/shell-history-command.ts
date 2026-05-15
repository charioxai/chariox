import type {
  HistoryEvent,
  SemanticHistoryMatch,
} from "./kernel-types.js"
import {
  searchHistoryRequest,
  semanticSearchHistoryRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatHistoryEvents,
  formatSemanticHistoryMatches,
} from "./shell-history-format.js"

type ShellHistoryCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeHistoryCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellHistoryCommandDeps,
): Promise<ShellCommandResult> {
  const [action, ...rest] = parsed.args
  if (action !== "search" && action !== "semantic-search") {
    return { ok: false, message: "usage: history search <query> | history semantic-search [--agent] <query>" }
  }
  const semanticMode = action === "semantic-search" && rest[0] === "--agent" ? "agent" : "knn"
  const queryArgs = semanticMode === "agent" ? rest.slice(1) : rest
  const query = queryArgs.join(" ").trim()
  if (!query) {
    return { ok: false, message: `usage: history ${action} <query>` }
  }
  const filters = {
    session_id: context.sessionId ?? null,
    limit: action === "semantic-search" ? 20 : 50,
  }
  if (action === "semantic-search") {
    const response = await deps.client.send(semanticSearchHistoryRequest(query, { ...filters, mode: semanticMode }))
    const payload = expectVariant<{
      results?: SemanticHistoryMatch[]
      unavailable_reason?: string | null
      answer?: string | null
    }>(response, "SemanticHistoryEvents")
    const unavailable = payload.unavailable_reason?.trim()
    if (unavailable) {
      return { ok: false, message: unavailable }
    }
    return {
      ok: true,
      message: [payload.answer?.trim(), formatSemanticHistoryMatches(payload.results ?? [])].filter(Boolean).join("\n\n"),
      format: "text",
    }
  }
  const response = await deps.client.send(searchHistoryRequest(query, filters))
  const payload = expectVariant<{ events?: HistoryEvent[] }>(response, "HistoryEvents")
  return {
    ok: true,
    message: formatHistoryEvents(payload.events ?? []),
    format: "text",
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

import type {
  RecallEvent,
  SemanticRecallMatch,
} from "./kernel-types.js"
import {
  searchRecallRequest,
  semanticSearchRecallRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatRecallEvents,
  formatSemanticRecallMatches,
} from "./shell-recall-format.js"

type ShellRecallCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeRecallCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellRecallCommandDeps,
): Promise<ShellCommandResult> {
  const [action, ...rest] = parsed.args
  if (action !== "search" && action !== "semantic-search") {
    return { ok: false, message: "usage: recall search <query> | recall semantic-search [--agent] <query>" }
  }
  const semanticMode = action === "semantic-search" && rest[0] === "--agent" ? "agent" : "knn"
  const queryArgs = semanticMode === "agent" ? rest.slice(1) : rest
  const query = queryArgs.join(" ").trim()
  if (!query) {
    return { ok: false, message: `usage: recall ${action} <query>` }
  }
  const filters = {
    session_id: context.sessionId ?? null,
    limit: action === "semantic-search" ? 20 : 50,
  }
  if (action === "semantic-search") {
    const response = await deps.client.send(semanticSearchRecallRequest(query, { ...filters, mode: semanticMode }))
    const payload = expectVariant<{
      results?: SemanticRecallMatch[]
      unavailable_reason?: string | null
      answer?: string | null
    }>(response, "SemanticRecallEvents")
    const unavailable = payload.unavailable_reason?.trim()
    if (unavailable) {
      return { ok: false, message: unavailable }
    }
    return {
      ok: true,
      message: [payload.answer?.trim(), formatSemanticRecallMatches(payload.results ?? [])].filter(Boolean).join("\n\n"),
      format: "text",
    }
  }
  const response = await deps.client.send(searchRecallRequest(query, filters))
  const payload = expectVariant<{ events?: RecallEvent[] }>(response, "RecallEvents")
  return {
    ok: true,
    message: formatRecallEvents(payload.events ?? []),
    format: "text",
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

import { sessionHasActivePrompt } from "@chariox/kernel-client/session-prompt-identity"
import type { RuntimeSession, TerminalOutputRecord } from "../cli-types.js"
import { normalizeRuntimeSessionWithAgentActivity } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { cancelActivePromptRequest, getSessionStateRequest } from "../ipc-requests.js"
import { getSessionHistoryBlobContent, getSessionHistoryOutline } from "../session-history-api.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"

// MP-08/MP-11: a display thread is not the executing provider thread. Native
// stop uses the same kernel cancellation path as every other Chariox client.
export async function handleCodexNativeTurnInterrupt(
  message: CodexJsonRpcMessage,
  options: { client: LocalIpcClient; sessionId: string; attachmentId: string; agentId: string; promptId: string | null;
    record?: Pick<TerminalOutputRecord, "timestamp_ms" | "kind" | "merge_key" | "source_attachment_id"> | null;
    debug?: (label: string, payload: unknown) => void },
  sendClient: (message: unknown) => void,
) {
  try {
    const promptId = options.promptId ?? await findCodexOutputPrompt(options)
    if (!promptId) throw new Error("Unknown native turn")
    const response = await options.client.send<Record<string, unknown>>(getSessionStateRequest(options.sessionId))
    const session = normalizeRuntimeSessionWithAgentActivity(response.SessionState as { session: RuntimeSession })
    options.debug?.("native_interrupt_correlated", {
      promptId, activePromptId: session.prompt_states?.[options.agentId]?.active_prompt?.id,
      pendingPromptId: session.prompt_states?.[options.agentId]?.active_prompt?.pending_prompt_id,
      activeTurnPromptId: session.agent_activity?.[options.agentId]?.active_turn?.prompt_id,
      matches: sessionHasActivePrompt(session, options.agentId, promptId),
    })
    // A delayed Escape for a completed turn must not cancel its successor.
    if (sessionHasActivePrompt(session, options.agentId, promptId)) {
      await options.client.send(cancelActivePromptRequest(options.sessionId, options.attachmentId, options.agentId))
    }
    sendClient({ id: message.id, result: {} })
  } catch (error) {
    options.debug?.("native_interrupt_failed", { error: error instanceof Error ? error.message : String(error) })
    sendClient({ id: message.id, error: { code: -32000, message: error instanceof Error ? error.message : String(error) } })
  }
}

// Ordinary local output may omit prompt_id. Its provider item key and time
// identify the owning turn in durable kernel history, without guessing that
// whatever is currently active owns an old display item.
async function findCodexOutputPrompt(options: {
  client: LocalIpcClient; sessionId: string; agentId: string;
  record?: Pick<TerminalOutputRecord, "timestamp_ms" | "kind" | "merge_key" | "source_attachment_id"> | null;
}): Promise<string | null> {
  const record = options.record
  if (!record?.merge_key || !Number.isFinite(record.timestamp_ms)) return null
  let cursor: import("../cli-types.js").SessionHistoryOutlineCursor | null = null
  do {
    const outline = await getSessionHistoryOutline(options.client, options.sessionId, [options.agentId], 100, cursor)
    const agent = outline.agents.find((candidate) => candidate.agent_id === options.agentId)
    if (!agent) return null
    const matches: string[] = []
    for (const turn of agent.turns) {
      if (!turn.prompt_id || turn.started_at_ms > record.timestamp_ms
        || (turn.completed_at_ms !== null && turn.completed_at_ms < record.timestamp_ms)) continue
      const entries = [...turn.entries, ...(turn.summary ? [turn.summary] : [])]
      for (const blob of turn.blobs) {
        const content = await getSessionHistoryBlobContent(options.client, options.sessionId, options.agentId, blob.blob_id)
        entries.push(...content.entries)
      }
      if (entries.some(({ entry }) => entry.merge_key === record.merge_key && entry.kind === record.kind
        && entry.source_attachment_id === record.source_attachment_id)) matches.push(turn.prompt_id)
    }
    if (matches.length) return matches.length === 1 ? matches[0]! : null
    const next = agent.next_cursor ?? null
    if (next && cursor && next.before_sequence >= cursor.before_sequence) throw new Error("Kernel history cursor did not advance")
    cursor = next
  } while (cursor)
  return null
}

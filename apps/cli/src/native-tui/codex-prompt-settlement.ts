import type { LocalIpcClient } from "../ipc.js"
import { getSessionHistoryOutline } from "../session-history-api.js"
import type { SessionHistoryOutlineCursor } from "../cli-types.js"

export type CodexPromptSettlement = { completedAtMs: number; status: "completed" | "cancelled" | "failed" }

// MP-08/MP-11: recover lifecycle by prompt identity, including completions
// overwritten in the latest session snapshot while the native pump stalled.
export async function loadCodexPromptSettlements(
  client: LocalIpcClient, sessionId: string, agentId: string, promptIds: string[],
): Promise<Map<string, CodexPromptSettlement>> {
  const remaining = new Set(promptIds)
  const settled = new Map<string, CodexPromptSettlement>()
  let cursor: SessionHistoryOutlineCursor | null = null
  do {
    const outline = await getSessionHistoryOutline(client, sessionId, [agentId], 100, cursor)
    const agent = outline.agents.find((candidate) => candidate.agent_id === agentId)
    if (!agent) break
    for (const turn of agent.turns) {
      if (!turn.prompt_id || !remaining.has(turn.prompt_id)) continue
      remaining.delete(turn.prompt_id)
      if ((turn.lifecycle === "completed" || turn.lifecycle === "cancelled" || turn.lifecycle === "failed") && Number.isFinite(turn.completed_at_ms)) {
        settled.set(turn.prompt_id, { completedAtMs: turn.completed_at_ms!, status: turn.lifecycle })
      }
    }
    const next = agent.next_cursor ?? null
    if (next && cursor && next.before_sequence >= cursor.before_sequence) throw new Error("Kernel history cursor did not advance")
    cursor = next
  } while (remaining.size && cursor)
  return settled
}

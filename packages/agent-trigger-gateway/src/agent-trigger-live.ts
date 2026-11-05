import type { TerminalOutputEntry, TerminalTurnState } from "@chariox/transcript-view"
import type { Conversation } from "./agent-trigger-client.js"

/** What the trigger page has shown so far; history snapshots replace it turn by turn. */
export type LiveState = { entries: Map<string, TerminalOutputEntry>; turns: Map<string, TerminalTurnState>; decoders: Map<string, TextDecoder> }

const STEERING_PROMPT_MERGE_KEY_PREFIX = "steering-prompt:"
const roles = { prompt_echo: "user", provider_output: "assistant", provider_reasoning: "reasoning", provider_tool: "tool", provider_error: "error", provider_status: "status" } as const

/** Drops live entries and turns that an authoritative history snapshot now covers. */
export function reconcileSnapshot(state: LiveState, value: Conversation) {
  const ids = new Set(value.turns.map(turn => turn.id))
  const promptIds = new Set(value.turns.map(turn => turn.kernelPromptId).filter(Boolean))
  for (const [id, turn] of state.turns) {
    if (id.startsWith("live:") && turn.kernelPromptId && promptIds.has(turn.kernelPromptId)) { ids.add(id); state.turns.delete(id) }
  }
  for (const [key, entry] of state.entries) if (entry.turnId && ids.has(entry.turnId)) state.entries.delete(key)
  for (const key of state.decoders.keys()) if ([...promptIds].some(id => key.startsWith(`live:${id}:`))) state.decoders.delete(key)
  for (const entry of value.entries) state.entries.set(entry.key, entry)
  for (const turn of value.turns) state.turns.set(turn.id, { ...turn, expanded: state.turns.get(turn.id)?.expanded ?? turn.expanded })
}

/**
 * A steering echo carries the steering prompt's own id, but history stores the
 * message under the turn it steers. It joins that running turn instead of
 * opening a turn of its own, so the next snapshot replaces it.
 */
function steeredTurn(turns: Map<string, TerminalTurnState>, agentId: string): TerminalTurnState | undefined {
  return [...turns.values()].filter(turn => turn.agentId === agentId && !turn.kernelSettled).sort((a, b) => b.startedAtMs - a.startedAtMs)[0]
}

export function applyOutputRecords(state: LiveState, records: Array<Record<string, unknown>>) {
  for (const record of records) {
    const role = roles[record.kind as keyof typeof roles]
    // Session-level records such as provider status carry no prompt, so they belong to no turn.
    if (!role || !Array.isArray(record.bytes) || typeof record.agent_id !== "string" || typeof record.prompt_id !== "string") continue
    const mergeKey = typeof record.merge_key === "string" ? record.merge_key : undefined
    const steering = mergeKey?.startsWith(STEERING_PROMPT_MERGE_KEY_PREFIX) === true
    const echoedPromptId = record.prompt_id
    const steered = steering ? steeredTurn(state.turns, record.agent_id) : undefined
    const promptId = steered?.kernelPromptId ?? echoedPromptId
    const turn = steered ?? [...state.turns.values()].find(turn => turn.kernelPromptId === promptId) ?? {
      id: `live:${promptId}`, agentId: record.agent_id, kernelPromptId: promptId, startedAtMs: Number(record.timestamp_ms),
      lastEntryAtMs: Number(record.timestamp_ms), kernelObservedActive: true, expanded: true,
    }
    state.turns.set(turn.id, turn)
    const merge = mergeKey ?? String(record.record_id ?? record.sequence ?? Date.now())
    const key = `live:${promptId}:${role}:${merge}`
    const previous = state.entries.get(key)
    const decoder = state.decoders.get(key) ?? new TextDecoder()
    state.decoders.set(key, decoder)
    const text = decoder.decode(new Uint8Array(record.bytes as number[]), { stream: true })
    state.entries.set(key, { key, turnId: turn.id, agentId: record.agent_id, role,
      createdAtMs: Number(record.timestamp_ms), promptId,
      ...(role === "user" && typeof record.prompt_origin === "string" ? { promptOrigin: record.prompt_origin } : {}), text: role === "assistant" || role === "reasoning" ? (previous?.text ?? "") + text : text })
  }
}

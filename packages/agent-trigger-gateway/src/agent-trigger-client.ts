import { randomUUID } from "node:crypto"
import type { KernelClient } from "./types.js"
import type { AgentTrigger, SessionHistoryPageEntry, SessionHistoryOutlineTurn, TriggerDelivery } from "./types.js"
import type { Unconfirmed } from "./agent-trigger-unconfirmed.js"
import type { TerminalOutputEntry, TerminalOutputRole, TerminalTurnState } from "@chariox/transcript-view"

export type Invocation = { status: "accepted"; prompt_id: string; outcome: "started" | "queued" | "steered" }
  | { status: "offline" | "disabled" | "busy" | "payload_too_large" }
type Entry = SessionHistoryPageEntry
type WireTurn = SessionHistoryOutlineTurn
type Outline = { agents: Array<{ agent_id: string; turns: WireTurn[]; next_cursor?: { before_sequence: number } | null }> }
type ReadPayload = { trigger: AgentTrigger; agent_alias: string; online: boolean; busy: boolean; history: { SessionHistoryOutline: Outline }; deliveries: TriggerDelivery[] }
export type Conversation = { alias: string; online: boolean; busy: boolean; enabled: boolean;
  entries: TerminalOutputEntry[]; turns: TerminalTurnState[]; nextCursor: { before_sequence: number } | null; unconfirmed: Unconfirmed[] }
const roles: Record<string, TerminalOutputRole> = { user_prompt: "user", provider_output: "assistant", provider_reasoning: "reasoning",
  provider_tool: "tool", provider_error: "error", provider_status: "status", notice: "status" }

/** Each viewer owns its kernel attachment, never a provider connection. */
export class AgentTriggerClient {
  readonly kernel: KernelClient
  private readonly clientId = `agent-http:${randomUUID()}`
  private attachmentId: string | null = null
  private agentId: string | null = null
  constructor(readonly sessionId: string, readonly triggerId: string, kernel: KernelClient) { this.kernel = kernel }
  async metadata(cursor: { before_sequence: number } | null = null): Promise<ReadPayload> {
    const response = await this.kernel.send<{ AgentTriggerTranscript: ReadPayload }>({ ReadAgentTriggerTranscript: { session_id: this.sessionId, trigger_id: this.triggerId, cursor } })
    if (!response.AgentTriggerTranscript) throw Error("HTTP trigger was not found")
    this.agentId = response.AgentTriggerTranscript.trigger.agent_id
    return response.AgentTriggerTranscript
  }
  async subscribe(): Promise<void> {
    const response = await this.kernel.send<{ SessionAttached: { attachment: { id: string } } }>({ AttachToSession: {
      session_id: this.sessionId, client_id: this.clientId, capability_level: "InteractiveStructured",
    } })
    this.attachmentId = response.SessionAttached.attachment.id
    await this.kernel.subscribeToKernelEvents(this.sessionId, this.attachmentId)
  }
  async invoke(payload: unknown): Promise<Invocation> {
    const response = await this.kernel.send<{ AgentTriggerInvoked: { invocation: Invocation } }>({ InvokeAgentTrigger: { session_id: this.sessionId, trigger_id: this.triggerId, payload } })
    return response.AgentTriggerInvoked.invocation
  }
  /** Sends a firing the provider did not confirm once more; the kernel allows it once. */
  async resend(promptId: string): Promise<void> {
    await this.kernel.send({ ResendTriggerDelivery: { session_id: this.sessionId, prompt_id: promptId } })
  }
  async conversation(cursor: { before_sequence: number } | null = null): Promise<Conversation> {
    const meta = await this.metadata(cursor)
    const outline = meta.history.SessionHistoryOutline.agents.find(a => a.agent_id === meta.trigger.agent_id)
    const turns = outline?.turns ?? []
    const blobIds = [...new Set(turns.flatMap(turn => turn.blobs.map(blob => blob.blob_id)))]
    const blobs = new Map<string, Entry[]>()
    for (let offset = 0; offset < blobIds.length; offset += 64) {
      const response = await this.kernel.send<{ SessionHistoryBlobContents: { blobs: Array<{ blob_id: string; entries: Entry[] }> } }>({
        GetSessionHistoryBlobContents: { session_id: this.sessionId, agent_id: meta.trigger.agent_id, blob_ids: blobIds.slice(offset, offset + 64) },
      })
      for (const blob of response.SessionHistoryBlobContents.blobs) blobs.set(blob.blob_id, blob.entries)
    }
    const entries = agentTriggerHistoryEntries(meta.trigger.agent_id, turns, blobs)
    return { alias: meta.agent_alias, online: meta.online, busy: meta.busy, enabled: meta.trigger.enabled, entries,
      turns: turns.map(turn => ({ id: turn.turn_id, agentId: meta.trigger.agent_id, startedAtMs: turn.started_at_ms,
        lastEntryAtMs: turn.completed_at_ms ?? turn.started_at_ms, kernelObservedActive: turn.lifecycle === "open", historyObserved: true,
        ...(turn.prompt_id ? { kernelPromptId: turn.prompt_id } : {}),
        ...(turn.completed_at_ms != null ? { completedAtMs: turn.completed_at_ms } : {}),
        ...(turn.lifecycle !== "open" ? { kernelSettled: true, kernelSettlementStatus: historySettlement(turn.lifecycle) } : {}), expanded: false })),
      nextCursor: outline?.next_cursor ?? null,
      unconfirmed: meta.deliveries.map(({ prompt_id, origin, prompt }) => ({ prompt_id, origin, prompt })) }
  }
  onChange(callback: (records?: Array<Record<string, unknown>>) => void): () => void {
    return this.kernel.onKernelEvent(event => {
      if (event.event === "terminal_output" && Array.isArray(event.records) && event.records.some(record => record.agent_id === this.agentId)) callback(event.records.filter(record => record.agent_id === this.agentId))
      else if (event.event === "session_snapshot" || event.event === "assistant_message_completed" || event.event === "runtime_notices") callback()
    })
  }
  async close(): Promise<void> {
    if (this.attachmentId) await this.kernel.send({ DetachFromSession: { attachment_id: this.attachmentId } }).catch(() => {})
    await this.kernel.close()
  }
}

/**
 * The canonical history entry index is shared by prompts, summaries and blob rows.
 * The outline summary is the turn's final assistant message, cut at the inline
 * limit; its events are the provider output at or after the summary's index,
 * so their blob rows rebuild the full text and earlier output stays as it is.
 */
export function agentTriggerHistoryEntries(agentId: string, turns: WireTurn[], blobs: ReadonlyMap<string, Entry[]>): TerminalOutputEntry[] {
  const entries: TerminalOutputEntry[] = []
  for (const turn of turns) {
    const rows = new Map<number, Entry>()
    for (const row of [turn.user_prompt, ...turn.entries, ...turn.blobs.flatMap(b => blobs.get(b.blob_id) ?? [])]) rows.set(row.entry_index, row)
    if (turn.summary) {
      const first = turn.summary.entry_index
      const finalOutput = [...rows.values()].filter(row => row.entry.kind === "provider_output" && row.entry_index >= first).sort((a, b) => a.entry_index - b.entry_index)
      const full = finalOutput.map(row => row.entry.text).join("")
      for (const row of finalOutput) rows.delete(row.entry_index)
      const complete = [...full].length === turn.summary.total_chars
      rows.set(first, complete ? { ...turn.summary, entry: { ...turn.summary.entry, text: full }, fragment_end: turn.summary.total_chars } : turn.summary)
    }
    for (const row of [...rows.values()].sort((a, b) => a.entry_index - b.entry_index)) {
      entries.push({ key: `${row.entry_index}`, role: roles[row.entry.kind] ?? "status", agentId: agentId,
        turnId: turn.turn_id, createdAtMs: row.entry.timestamp_ms ?? turn.started_at_ms, text: row.entry.text, promptId: turn.prompt_id ?? null,
        ...(row.entry.kind === "user_prompt" && row.entry.prompt_origin ? { promptOrigin: row.entry.prompt_origin } : {}),
        ...(turn.summary?.entry_index === row.entry_index ? { historyTurnSummary: true } : {}) })
    }
  }
  return entries
}

function historySettlement(lifecycle: string): "completed" | "cancelled" | "failed" {
  if (lifecycle === "completed" || lifecycle === "cancelled" || lifecycle === "failed") return lifecycle
  throw Error(`Unsupported settled history lifecycle: ${lifecycle}`)
}

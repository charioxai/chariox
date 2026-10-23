import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { readAgentTriggerTranscriptRequest, invokeAgentTriggerRequest } from "@chariox/kernel-client/ipc-requests"
import type { AgentTrigger } from "@chariox/kernel-client/kernel-types"
import type { TerminalOutputEntry, TerminalOutputRole, TerminalTurnState } from "@chariox/transcript-view"

export type Invocation = { status: "accepted"; prompt_id: string; outcome: "started" | "queued" | "steered" }
  | { status: "offline" | "disabled" | "busy" | "payload_too_large" }
type Entry = { sequence: number; entry: { kind: string; text: string; timestamp_ms: number; agent_id?: string | null; merge_key?: string | null } }
type Blob = { blob_id: string }
type WireTurn = { turn_id: string; prompt_id?: string | null; started_at_ms: number; completed_at_ms?: number | null;
  lifecycle: "open" | "completed" | "cancelled" | "failed"; user_prompt: Entry; summary?: Entry | null; entries: Entry[]; blobs: Blob[] }
type Outline = { agents: Array<{ agent_id: string; turns: WireTurn[]; next_cursor?: { before_sequence: number } | null }> }
type ReadPayload = { trigger: AgentTrigger; agent_alias: string; online: boolean; busy: boolean; history: { SessionHistoryOutline: Outline } }
export type Conversation = { alias: string; online: boolean; busy: boolean; enabled: boolean;
  entries: TerminalOutputEntry[]; turns: TerminalTurnState[]; nextCursor: { before_sequence: number } | null }
const roles: Record<string, TerminalOutputRole> = { user_prompt: "user", provider_output: "assistant", provider_reasoning: "reasoning",
  provider_tool: "tool", provider_error: "error", provider_status: "status", notice: "status" }

/** The gateway has one normal local kernel connection, never a provider connection. */
export class AgentTriggerClient {
  readonly kernel: LocalIpcClient
  private attachmentId: string | null = null
  private agentId: string | null = null
  constructor(readonly sessionId: string, readonly triggerId: string, endpoint: string) {
    this.kernel = new LocalIpcClient(endpoint, { controlRequestRetryDeadlineMs: 5000 })
  }
  async metadata(cursor: { before_sequence: number } | null = null): Promise<ReadPayload> {
    const response = await this.kernel.send<{ AgentTriggerTranscript: ReadPayload }>(readAgentTriggerTranscriptRequest(this.sessionId, this.triggerId, cursor))
    if (!response.AgentTriggerTranscript) throw Error("HTTP trigger was not found")
    this.agentId = response.AgentTriggerTranscript.trigger.agent_id
    return response.AgentTriggerTranscript
  }
  async subscribe(): Promise<void> {
    const response = await this.kernel.send<{ SessionAttached: { attachment: { id: string } } }>({ AttachToSession: {
      session_id: this.sessionId, client_id: `agent-http:${this.triggerId}`, capability_level: "InteractiveStructured",
    } })
    this.attachmentId = response.SessionAttached.attachment.id
    await this.kernel.subscribeToKernelEvents(this.sessionId, this.attachmentId)
  }
  async invoke(payload: unknown): Promise<Invocation> {
    const response = await this.kernel.send<{ AgentTriggerInvoked: { invocation: Invocation } }>(invokeAgentTriggerRequest(this.sessionId, this.triggerId, payload))
    return response.AgentTriggerInvoked.invocation
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
    const entries: TerminalOutputEntry[] = []
    for (const turn of turns) {
      const rows = new Map<number, Entry>()
      for (const row of [turn.user_prompt, ...turn.entries, ...turn.blobs.flatMap(b => blobs.get(b.blob_id) ?? []), ...(turn.summary ? [turn.summary] : [])]) rows.set(row.sequence, row)
      for (const row of [...rows.values()].sort((a,b) => a.sequence - b.sequence)) {
        if (turn.summary && row.entry.kind === "provider_output" && row.sequence !== turn.summary.sequence) continue
        entries.push({ key: `${row.sequence}`, role: roles[row.entry.kind] ?? "status", agentId: meta.trigger.agent_id,
          turnId: turn.turn_id, createdAtMs: row.entry.timestamp_ms, text: row.entry.text, promptId: turn.prompt_id ?? null,
          ...(turn.summary?.sequence === row.sequence ? { historyTurnSummary: true } : {}) })
      }
    }
    return { alias: meta.agent_alias, online: meta.online, busy: meta.busy, enabled: meta.trigger.enabled, entries,
      turns: turns.map(turn => ({ id: turn.turn_id, agentId: meta.trigger.agent_id, startedAtMs: turn.started_at_ms,
        lastEntryAtMs: turn.completed_at_ms ?? turn.started_at_ms, kernelObservedActive: turn.lifecycle === "open", historyObserved: true,
        ...(turn.prompt_id ? { kernelPromptId: turn.prompt_id } : {}),
        ...(turn.completed_at_ms != null ? { completedAtMs: turn.completed_at_ms } : {}),
        ...(turn.lifecycle !== "open" ? { kernelSettled: true, kernelSettlementStatus: turn.lifecycle } : {}), expanded: false })),
      nextCursor: outline?.next_cursor ?? null }
  }
  onChange(callback: (records?: Array<Record<string, unknown>>) => void): () => void {
    return this.kernel.onKernelEvent(event => {
      if (event.event === "terminal_output" && event.records.some(record => record.agent_id === this.agentId)) callback(event.records.filter(record => record.agent_id === this.agentId))
      else if (event.event === "session_snapshot" || event.event === "assistant_message_completed" || event.event === "runtime_notices") callback()
    })
  }
  async close(): Promise<void> {
    if (this.attachmentId) await this.kernel.send({ DetachFromSession: { attachment_id: this.attachmentId } }).catch(() => {})
    await this.kernel.close()
  }
}

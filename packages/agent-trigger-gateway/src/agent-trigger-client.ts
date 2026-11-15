import { randomUUID } from "node:crypto"
import type { KernelClient } from "./types.js"
import type { AgentTrigger, SessionHistoryPageEntry, SessionHistoryOutlineTurn, TriggerDelivery } from "./types.js"
import type { Unconfirmed } from "./agent-trigger-unconfirmed.js"
import type { TerminalOutputEntry, TerminalOutputRole, TerminalTurnState } from "@chariox/transcript-view"

import type { AgentHttpOptions, HttpBody, Invocation, TriggerReply } from "./agent-trigger-http.js"
export type { Invocation } from "./agent-trigger-http.js"
type Entry = SessionHistoryPageEntry
type WireTurn = SessionHistoryOutlineTurn
type Outline = { agents: Array<{ agent_id: string; turns: WireTurn[]; next_cursor?: { before_sequence: number } | null }> }
type ReadPayload = { trigger: AgentTrigger; agent_alias: string; online: boolean; busy: boolean; history: { SessionHistoryOutline: Outline }; deliveries: TriggerDelivery[] }
export type Conversation = { alias: string; online: boolean; busy: boolean; enabled: boolean;
  entries: TerminalOutputEntry[]; turns: TerminalTurnState[]; nextCursor: { before_sequence: number } | null; unconfirmed: Unconfirmed[] }
const roles: Record<string, TerminalOutputRole> = { user_prompt: "user", provider_output: "assistant", provider_reasoning: "reasoning",
  provider_tool: "tool", provider_error: "error", provider_status: "status", notice: "status" }

const maxWorkspaceImageBytes = 8 * 1024 * 1024
/** A workspace-relative path; an absolute one only inside the worktree. Never `..`, empty or control characters. */
export function workspaceImagePath(reference: string, worktree: string): string | null {
  const root = worktree.replace(/\/+$/, "")
  const relative = reference.startsWith("/") ? reference.startsWith(`${root}/`) ? reference.slice(root.length + 1) : null : reference.replace(/^(?:\.\/)+/, "")
  if (!relative || /[\x00-\x1f\x7f\\]/.test(relative) || relative.split("/").some(part => !part || part === "." || part === "..")) return null
  return relative
}
const imageMime = (bytes: Buffer): string | null =>
  bytes.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) ? "image/png"
    : bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff ? "image/jpeg"
    : bytes.subarray(0, 6).toString("latin1").match(/^GIF8[79]a$/) ? "image/gif"
    : bytes.subarray(0, 4).toString("latin1") === "RIFF" && bytes.subarray(8, 12).toString("latin1") === "WEBP" ? "image/webp"
    : null

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
  async invoke(input: unknown, http: HttpBody | null): Promise<Invocation> {
    const response = await this.kernel.send<{ AgentTriggerInvoked: { invocation: Invocation } }>({ InvokeAgentTrigger: { session_id: this.sessionId, trigger_id: this.triggerId, input, ...(http ? { http } : {}) } })
    return response.AgentTriggerInvoked.invocation
  }
  async options(): Promise<{ enabled: boolean; options: AgentHttpOptions }> {
    const response = await this.kernel.send<{ AgentTriggerHttpOptionsRead: { enabled: boolean; options: AgentHttpOptions } }>({ ReadAgentTriggerHttpOptions: { session_id: this.sessionId, trigger_id: this.triggerId } })
    return response.AgentTriggerHttpOptionsRead
  }
  async reply(promptId: string): Promise<TriggerReply> {
    const response = await this.kernel.send<{ AgentTriggerReplyRead: { reply: TriggerReply } }>({ ReadAgentTriggerReply: { session_id: this.sessionId, trigger_id: this.triggerId, prompt_id: promptId } })
    return response.AgentTriggerReplyRead.reply
  }
  /** Sends a firing the provider did not confirm once more; the kernel allows it once. */
  async resend(promptId: string): Promise<void> {
    await this.kernel.send({ ResendTriggerDelivery: { session_id: this.sessionId, prompt_id: promptId } })
  }
  async conversation(cursor: { before_sequence: number } | null = null): Promise<Conversation> {
    const meta = await this.metadata(cursor)
    const outline = meta.history.SessionHistoryOutline.agents.find(a => a.agent_id === meta.trigger.agent_id)
    const turns = outline?.turns ?? []
    const blobIds = [...new Set(turns.flatMap(turn => [...turn.blobs.map(blob => blob.blob_id), ...(promptIsCut(turn) ? [promptBlobId(turn)] : [])]))]
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
  /**
   * A picture a reply references by a path in the agent's workspace, read through the kernel like every other reader of the Room.
   * Null when the path leaves the workspace, the file is missing or too large, or it is not a PNG, JPEG, GIF or WebP image.
   */
  async workspaceImage(reference: string): Promise<{ mime: string; bytes: Buffer } | null> {
    if (!this.agentId) await this.metadata()
    type Agent = { id: string; workspace_id?: string | null; worktree_id?: string | null }
    const state = await this.kernel.send<{ SessionState?: { session?: { workspace_id: string; worktree_id: string; agents: Agent[] } } }>({ GetSessionState: { session_id: this.sessionId } })
    const session = state.SessionState?.session, agent = session?.agents?.find(item => item.id === this.agentId)
    const workspace = agent?.workspace_id?.trim() || session?.workspace_id.trim()
    const worktree = agent?.worktree_id?.trim() || session?.worktree_id.trim()
    const path = workspace && worktree ? workspaceImagePath(reference, worktree) : null
    if (!workspace || !worktree || !path) return null
    const response = await this.kernel.send<{ WorkspaceFileContent?: { content?: { content_base64?: string | null; truncated?: boolean; size_bytes?: number } } }>({
      GetWorkspaceFileContent: { workspace_id: workspace, worktree_id: worktree, path, compare_ref: null, known_fingerprint: null, max_bytes: maxWorkspaceImageBytes },
    })
    const content = response.WorkspaceFileContent?.content
    if (!content?.content_base64 || content.truncated || (content.size_bytes ?? 0) > maxWorkspaceImageBytes) return null
    const bytes = Buffer.from(content.content_base64, "base64"), mime = imageMime(bytes)
    return mime ? { mime, bytes } : null
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
    // The outline cuts a prompt at the inline limit and never lists it in `blobs`; its own blob holds the whole text.
    const fullPrompt = promptIsCut(turn) ? (blobs.get(promptBlobId(turn)) ?? []).filter(row => row.entry_index === turn.user_prompt.entry_index && row.fragment_end === row.total_chars) : []
    for (const row of [turn.user_prompt, ...fullPrompt, ...turn.entries, ...turn.blobs.flatMap(b => blobs.get(b.blob_id) ?? [])]) rows.set(row.entry_index, row)
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

function promptIsCut(turn: WireTurn): boolean {
  return (turn.user_prompt.fragment_end ?? turn.user_prompt.total_chars) < turn.user_prompt.total_chars
}

/** A history entry's index is its sequence, and a blob id names a sequence range. */
function promptBlobId(turn: WireTurn): string {
  return `history:${turn.user_prompt.entry_index}:${turn.user_prompt.entry_index}`
}

function historySettlement(lifecycle: string): "completed" | "cancelled" | "failed" {
  if (lifecycle === "completed" || lifecycle === "cancelled" || lifecycle === "failed") return lifecycle
  throw Error(`Unsupported settled history lifecycle: ${lifecycle}`)
}

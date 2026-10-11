import type { RuntimeSession, TerminalOutputRecord } from "../cli-types.js"
import { isProviderIdleStatus } from "@chariox/kernel-client/provider-status"
import {
  terminalRecordTranscriptProjection,
  type TerminalRecordTranscriptProjection,
} from "@chariox/kernel-client/terminal-record-transcript"

type ProjectedItem = {
  key: string
  turnId: string
  itemId: string
  kind: "agentMessage" | "reasoning"
  text: string
  completedAtMs: number
  promptId: string | null
  record: Pick<TerminalOutputRecord, "timestamp_ms" | "kind" | "merge_key" | "source_attachment_id">
}

export function createCodexKernelOutputProjection(options: {
  agentId: string
  broadcast: (message: unknown) => void
  debug: (label: string, payload: unknown) => void
  resolveSettledPrompts?: (promptIds: string[]) => Promise<Map<string, number>>
  nowMs?: () => number
}) {
  const nowMs = options.nowMs ?? Date.now
  let projectedThreadId: string | null = null
  let nextProjectedTurnId = 1
  const projectedItems = new Map<string, ProjectedItem>()
  let projectedUserTurn: { turnId: string; promptId: string | null } | null = null

  const recordTimestampMs = (record: TerminalOutputRecord): number =>
    Number.isFinite(record.timestamp_ms) ? record.timestamp_ms : nowMs()

  const turnPayload = (turnId: string, status: "inProgress" | "completed", timestampMs: number) => ({
    id: turnId,
    items: [],
    itemsView: "notLoaded",
    status,
    error: null,
    startedAt: Math.floor(timestampMs / 1000),
    completedAt: status === "completed" ? Math.floor(timestampMs / 1000) : null,
    durationMs: null,
  })

  const startProjectedTurn = (timestampMs: number) => {
    if (!projectedThreadId) return null
    const turnId = `chariox-projected-turn-${nextProjectedTurnId++}`
    options.broadcast({
      jsonrpc: "2.0",
      method: "thread/status/changed",
      params: {
        threadId: projectedThreadId,
        status: { type: "active", activeFlags: [] },
      },
    })
    options.broadcast({
      jsonrpc: "2.0",
      method: "turn/started",
      params: {
        threadId: projectedThreadId,
        turn: turnPayload(turnId, "inProgress", timestampMs),
      },
    })
    return turnId
  }

  const completeProjectedItem = (projection: ProjectedItem) => {
    if (!projectedThreadId) return
    options.broadcast({
      jsonrpc: "2.0",
      method: "item/completed",
      params: {
        item: projection.kind === "reasoning"
          ? { type: "reasoning", id: projection.itemId, summary: [], content: [] }
          : { type: "agentMessage", id: projection.itemId, text: projection.text, phase: "final_answer", memoryCitation: null },
        threadId: projectedThreadId,
        turnId: projection.turnId,
        completedAtMs: projection.completedAtMs,
      },
    })
    options.broadcast({
      jsonrpc: "2.0",
      method: "thread/status/changed",
      params: {
        threadId: projectedThreadId,
        status: { type: "idle" },
      },
    })
    options.broadcast({
      jsonrpc: "2.0",
      method: "turn/completed",
      params: {
        threadId: projectedThreadId,
        turn: turnPayload(projection.turnId, "completed", projection.completedAtMs),
      },
    })
    projectedItems.delete(projection.key)
  }

  const project = (records: TerminalOutputRecord[]) => {
    options.debug("projection_batch_received", {
      agentId: options.agentId,
      projectedThreadId,
      count: records.length,
      records: records.map(summarizeProjectionRecord),
    })
    for (const record of records) {
      if (!projectedThreadId) {
        debugProjectionSkipped(options.debug, "missing_thread", options.agentId, record)
        continue
      }
      if (record.agent_id !== options.agentId) {
        debugProjectionSkipped(options.debug, "agent_mismatch", options.agentId, record)
        continue
      }
      const delta = Buffer.from(record.bytes).toString("utf8")
      if (!delta) {
        debugProjectionSkipped(options.debug, "empty_delta", options.agentId, record)
        continue
      }
      const recordProjection = terminalRecordTranscriptProjection(record, delta, {
        isProviderIdleStatus,
        shouldRenderProviderStatus: () => false,
      })
      if (!recordProjection.appendsLiveTranscript) {
        debugProjectionSkipped(options.debug, "not_live_transcript", options.agentId, record, {
          transcriptRole: recordProjection.transcriptRole,
          historyRefreshSignal: recordProjection.historyRefreshSignal,
          passiveExternalTelemetry: recordProjection.passiveExternalTelemetry,
          providerStatusIdle: recordProjection.providerStatusIdle,
          renderInAgentPane: recordProjection.renderInAgentPane,
        })
        continue
      }
      const timestampMs = recordTimestampMs(record)

      if (recordProjection.transcriptRole === "user") {
        const turnId = startProjectedTurn(timestampMs)
        if (!turnId) continue
        // Escape can arrive before the first assistant delta, while the TUI
        // still identifies the projected user echo as its executing turn.
        projectedUserTurn = { turnId, promptId: record.prompt_id ?? null }
        const itemId = `chariox-projected-user-${timestampMs}-${nextProjectedTurnId}`
        options.broadcast({
          jsonrpc: "2.0",
          method: "item/started",
          params: {
            item: {
              type: "userMessage",
              id: itemId,
              content: [{ type: "text", text: recordProjection.transcriptText, text_elements: [] }],
            },
            threadId: projectedThreadId,
            turnId,
            startedAtMs: timestampMs,
          },
        })
        options.broadcast({
          jsonrpc: "2.0",
          method: "item/completed",
          params: {
            item: {
              type: "userMessage",
              id: itemId,
              content: [{ type: "text", text: recordProjection.transcriptText, text_elements: [] }],
            },
            threadId: projectedThreadId,
            turnId,
            completedAtMs: timestampMs,
          },
        })
        options.debug("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
        continue
      }

      const itemKind = codexProjectedItemKind(recordProjection)
      if (!itemKind) {
        debugProjectionSkipped(options.debug, "unsupported_item_kind", options.agentId, record, {
          transcriptRole: recordProjection.transcriptRole,
        })
        continue
      }
      const itemKey = `${record.prompt_id ?? "unscoped"}:${itemKind}:${recordProjection.mergeKey ?? "default"}`
      let itemProjection = projectedItems.get(itemKey)
      if (!itemProjection) {
        const turnId = startProjectedTurn(timestampMs)
        if (!turnId) continue
        const itemId = `chariox-projected-${itemKind}-${timestampMs}-${nextProjectedTurnId}`
        itemProjection = {
          key: itemKey, turnId, itemId, kind: itemKind, text: "", completedAtMs: timestampMs, promptId: record.prompt_id ?? null,
          record: {
            timestamp_ms: timestampMs, kind: record.kind,
            ...(record.merge_key !== undefined ? { merge_key: record.merge_key } : {}),
            ...(record.source_attachment_id !== undefined ? { source_attachment_id: record.source_attachment_id } : {}),
          },
        }
        projectedItems.set(itemKey, itemProjection)
        options.broadcast({
          jsonrpc: "2.0",
          method: "item/started",
          params: {
            item: itemKind === "reasoning"
              ? { type: "reasoning", id: itemId, summary: [], content: [] }
              : { type: "agentMessage", id: itemId, text: "", phase: "final_answer", memoryCitation: null },
            threadId: projectedThreadId,
            turnId,
            startedAtMs: timestampMs,
          },
        })
      }
      itemProjection.text += recordProjection.transcriptText
      itemProjection.completedAtMs = Math.max(itemProjection.completedAtMs, timestampMs)
      options.broadcast({
        jsonrpc: "2.0",
        method: itemKind === "reasoning" ? "item/reasoning/textDelta" : "item/agentMessage/delta",
        params: {
          threadId: projectedThreadId,
          turnId: itemProjection.turnId,
          itemId: itemProjection.itemId,
          delta: recordProjection.transcriptText,
        },
      })
      if (record.kind === "provider_error") completeProjectedItem(itemProjection)
      options.debug("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
    }
  }

  return {
    project,
    promptIdForTurn: (turnId: string) => projectedUserTurn?.turnId === turnId
      ? projectedUserTurn.promptId
      : [...projectedItems.values()].find((item) => item.turnId === turnId)?.promptId ?? null,
    recordForTurn: (turnId: string) => [...projectedItems.values()].find((item) => item.turnId === turnId)?.record ?? null,
    projectSessionState: async (session: RuntimeSession) => {
      const activity = session.agent_activity?.[options.agentId]
      const completed = activity?.last_completed_turn
      // The snapshot is read before draining output. Its completion timestamp
      // fences unscoped legacy records if another turn starts during the drain.
      for (const item of projectedItems.values()) {
        if (!completed || (item.promptId ? item.promptId !== completed.prompt_id : item.completedAtMs > completed.completed_at_ms)) continue
        item.completedAtMs = completed.completed_at_ms
        completeProjectedItem(item)
      }
      // last_completed_turn is a latest-value projection, not a completion
      // log. Recover older outstanding IDs from kernel-owned durable history.
      // Exact IDs, rather than timestamps or idle state, fence newer turns.
      const promptState = session.prompt_states?.[options.agentId]
      const liveIds = new Set([promptState?.active_prompt?.id, ...(promptState?.queued_prompts ?? []).map((prompt) => prompt.id)])
      const unresolvedIds = [...new Set([...projectedItems.values()].flatMap((item) =>
        item.promptId && !liveIds.has(item.promptId) ? [item.promptId] : []))]
      if (!unresolvedIds.length || !options.resolveSettledPrompts) return
      const settlements = await options.resolveSettledPrompts(unresolvedIds)
      for (const item of projectedItems.values()) {
        const completedAt = item.promptId ? settlements.get(item.promptId) : undefined
        if (completedAt === undefined) continue
        item.completedAtMs = completedAt
        completeProjectedItem(item)
      }
    },
    setThreadId: (threadId: string) => {
      projectedThreadId = threadId
    },
  }
}

function codexProjectedItemKind(
  projection: TerminalRecordTranscriptProjection,
): ProjectedItem["kind"] | null {
  switch (projection.transcriptRole) {
    case "reasoning":
      return "reasoning"
    case "assistant":
    case "error":
      return "agentMessage"
    default:
      return null
  }
}

function debugProjectionSkipped(
  debug: (label: string, payload: unknown) => void,
  reason: string,
  expectedAgentId: string,
  record: TerminalOutputRecord,
  details: Record<string, unknown> = {},
) {
  debug("projection_record_skipped", {
    reason,
    expectedAgentId,
    ...summarizeProjectionRecord(record),
    ...details,
  })
}

function summarizeProjectionRecord(record: TerminalOutputRecord) {
  const text = Buffer.from(record.bytes).toString("utf8")
  return {
    recordId: record.record_id ?? null,
    agentId: record.agent_id ?? null,
    kind: record.kind,
    promptOrigin: record.prompt_origin ?? null,
    sourceAttachmentId: record.source_attachment_id ?? null,
    mergeKey: record.merge_key ?? null,
    byteLength: record.bytes.length,
    preview: text.slice(0, 96),
  }
}

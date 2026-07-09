import type { TerminalOutputRecord } from "../cli-types.js"
import { isProviderIdleStatus } from "@arroba/kernel-client/provider-status"
import {
  terminalRecordTranscriptProjection,
  type TerminalRecordTranscriptProjection,
} from "@arroba/kernel-client/terminal-record-transcript"

type ProjectedItem = {
  key: string
  turnId: string
  itemId: string
  kind: "agentMessage" | "reasoning"
  text: string
  completedAtMs: number
  timer: NodeJS.Timeout | null
}

export function createCodexKernelOutputProjection(options: {
  agentId: string
  broadcast: (message: unknown) => void
  debug: (label: string, payload: unknown) => void
  nowMs?: () => number
}) {
  const nowMs = options.nowMs ?? Date.now
  let projectedThreadId: string | null = null
  let nextProjectedTurnId = 1
  const projectedItems = new Map<string, ProjectedItem>()

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
    const turnId = `arroba-projected-turn-${nextProjectedTurnId++}`
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

  const completeProjectedItemSoon = (projection: ProjectedItem) => {
    if (projection.timer) clearTimeout(projection.timer)
    projection.timer = setTimeout(() => {
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
    }, 750)
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
        const itemId = `arroba-projected-user-${timestampMs}-${nextProjectedTurnId}`
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
      const itemKey = `${itemKind}:${recordProjection.mergeKey ?? "default"}`
      let itemProjection = projectedItems.get(itemKey)
      if (!itemProjection) {
        const turnId = startProjectedTurn(timestampMs)
        if (!turnId) continue
        const itemId = `arroba-projected-${itemKind}-${timestampMs}-${nextProjectedTurnId}`
        itemProjection = { key: itemKey, turnId, itemId, kind: itemKind, text: "", completedAtMs: timestampMs, timer: null }
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
      completeProjectedItemSoon(itemProjection)
      options.debug("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
    }
  }

  return {
    project,
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

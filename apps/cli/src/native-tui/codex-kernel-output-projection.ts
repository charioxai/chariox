import type { CodexPromptSettlement } from "./codex-prompt-settlement.js"
import { traceCodexLifecycle } from "./codex-lifecycle-trace.js"
import type { AgentRuntimeActivity, TerminalOutputRecord } from "../cli-types.js"
import { isProviderIdleStatus } from "@chariox/kernel-client/provider-status"
import {
  terminalRecordTranscriptProjection,
  type TerminalRecordTranscriptProjection,
} from "@chariox/kernel-client/terminal-record-transcript"

type ProjectedItem = {
  turnId: string
  itemId: string
  kind: "agentMessage" | "reasoning"
  text: string
}

export function createCodexKernelOutputProjection(options: {
  agentId: string
  broadcast: (message: unknown) => void
  debug: (label: string, payload: unknown) => void
  resolveSettledPrompts?: (promptIds: string[]) => Promise<Map<string, CodexPromptSettlement>>
  nowMs?: () => number
  onTurnMapped?: (threadId: string, turnId: string, promptId: string) => void
}) {
  const nowMs = options.nowMs ?? Date.now
  let projectedThreadId: string | null = null
  let nextProjectedItemId = 1
  const projectedItems = new Map<string, ProjectedItem>()
  const projectedTurns = new Map<string, { id: string, startedAtMs: number, completed: boolean }>()
  let activePromptId: string | null = null

  const recordTimestampMs = (record: TerminalOutputRecord): number =>
    Number.isFinite(record.timestamp_ms) ? record.timestamp_ms : nowMs()

  const turnPayload = (turnId: string, status: "inProgress" | "completed" | "interrupted" | "failed", timestampMs: number, completedAtMs?: number) => ({
    id: turnId,
    items: [],
    itemsView: "notLoaded",
    status,
    error: null,
    startedAt: Math.floor(timestampMs / 1000),
    completedAt: completedAtMs === undefined ? null : Math.floor(completedAtMs / 1000),
    durationMs: null,
  })

  const startProjectedTurn = (timestampMs: number, promptId?: string | null) => {
    if (!projectedThreadId) return null
    // MP-08 / MP-10 / MP-11: Native provider output can omit prompt_id.
    // It belongs to the admitted active home prompt, never an anonymous turn.
    const key = promptId ?? activePromptId
    if (!key) return null
    const existing = projectedTurns.get(key)
    if (existing) return existing.completed ? null : existing.id
    const turnId = `chariox-native-${key}`
    projectedTurns.set(key, { id: turnId, startedAtMs: timestampMs, completed: false })
    activePromptId = key
    options.onTurnMapped?.(projectedThreadId, turnId, key)
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

  // MP-08 / MP-10: Output silence says nothing about tool/approval waits.
  // Only the kernel's exact prompt settlement can complete its display turn.
  const settlePrompt = (completion: NonNullable<AgentRuntimeActivity["last_completed_turn"]>) => {
    traceCodexLifecycle("activity", { promptId: completion?.prompt_id,
      activePromptId, knownPromptIds: [...projectedTurns.keys()],
      settlement: completion?.settlement_status })
    if (!completion || completion.agent_id !== options.agentId || !projectedThreadId) return
    const turn = projectedTurns.get(completion.prompt_id)
    if (!turn || turn.completed) return
    for (const [key, item] of projectedItems) {
      if (item.turnId !== turn.id) continue
      options.broadcast({
        jsonrpc: "2.0", method: "item/completed",
        params: {
          item: item.kind === "reasoning"
            ? { type: "reasoning", id: item.itemId, summary: [], content: [] }
            : { type: "agentMessage", id: item.itemId, text: item.text, phase: "final_answer", memoryCitation: null },
          threadId: projectedThreadId, turnId: turn.id, completedAtMs: completion.completed_at_ms,
        },
      })
      projectedItems.delete(key)
    }
    traceCodexLifecycle("turn_completed", { promptId: completion.prompt_id, turnId: turn.id, threadId: projectedThreadId })
    turn.completed = true
    if (activePromptId === completion.prompt_id) {
      activePromptId = null
      options.broadcast({ jsonrpc: "2.0", method: "thread/status/changed",
        params: { threadId: projectedThreadId, status: { type: "idle" } } })
    }
    const status = completion.settlement_status === "cancelled" ? "interrupted" : completion.settlement_status
    options.broadcast({ jsonrpc: "2.0", method: "turn/completed", params: {
      threadId: projectedThreadId, turn: turnPayload(turn.id, status, turn.startedAtMs, completion.completed_at_ms),
    } })
    // Retain bounded tombstones so delayed records cannot reopen settled turns.
    if (projectedTurns.size > 64) {
      for (const [key, value] of projectedTurns) {
        if (projectedTurns.size <= 64) break
        if (value.completed) projectedTurns.delete(key)
      }
    }
  }

  const projectAgentActivity = async (activity: AgentRuntimeActivity | undefined) => {
    const completion = activity?.last_completed_turn
    if (completion) settlePrompt(completion)
    // MP-08/MP-11: a pump can miss more than one settlement. Recover only
    // known predecessor prompts; the current active prompt stays snapshot-owned.
    const predecessors = [...projectedTurns].filter(([id, turn]) =>
      !turn.completed && id !== activePromptId && id !== completion?.prompt_id).map(([id]) => id)
    if (!predecessors.length || !options.resolveSettledPrompts) return
    const threadId = projectedThreadId
    const settled = await options.resolveSettledPrompts(predecessors)
    if (projectedThreadId !== threadId) return
    for (const [promptId, value] of settled) {
      if (!predecessors.includes(promptId)) continue
      settlePrompt({ agent_id: options.agentId, prompt_id: promptId,
        completed_at_ms: value.completedAtMs, settlement_status: value.status,
        turn_id: promptId, provider_run_id: "", changed_paths: [], undo_available: false })
    }
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
        const turnId = recordProjection.steeringPrompt
          ? (activePromptId === null ? null : projectedTurns.get(activePromptId)?.id)
          : startProjectedTurn(timestampMs, record.prompt_id)
        if (!turnId) continue
        const itemId = `chariox-projected-user-${timestampMs}-${nextProjectedItemId++}`
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
      const itemKey = `${record.prompt_id ?? activePromptId ?? ""}:${itemKind}:${recordProjection.mergeKey ?? "default"}`
      let itemProjection = projectedItems.get(itemKey)
      if (!itemProjection) {
        const turnId = startProjectedTurn(timestampMs, record.prompt_id)
        if (!turnId) continue
        const itemId = `chariox-projected-${itemKind}-${timestampMs}-${nextProjectedItemId++}`
        itemProjection = { turnId, itemId, kind: itemKind, text: "" }
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
      options.debug("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
    }
  }

  // MP-08 / MP-10: An approval waits inside an active home turn. The official
  // TUI defers its overlay while an assistant item is streaming, so finish
  // only this prompt's preamble items before projecting the blocking request.
  const finishPromptItemsForInteraction = (promptId: string) => {
    const turn = projectedTurns.get(promptId)
    if (!projectedThreadId || !turn || turn.completed || activePromptId !== promptId) return
    for (const [key, item] of projectedItems) {
      if (item.turnId !== turn.id) continue
      options.broadcast({ jsonrpc: "2.0", method: "item/completed", params: {
        threadId: projectedThreadId, turnId: turn.id, completedAtMs: nowMs(),
        item: item.kind === "reasoning"
          ? { type: "reasoning", id: item.itemId, summary: [], content: [] }
          : { type: "agentMessage", id: item.itemId, text: item.text, phase: "commentary", memoryCitation: null },
      } })
      projectedItems.delete(key)
    }
  }

  return {
    project,
    finishPromptItemsForInteraction,
    projectAgentActivity,
    startPrompt: (promptId: string) => startProjectedTurn(nowMs(), promptId),
    setThreadId: (threadId: string) => {
      if (projectedThreadId !== threadId) {
        projectedItems.clear()
        projectedTurns.clear()
        activePromptId = null
      }
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

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
  timer: NodeJS.Timeout | null
}

export function createCodexKernelOutputProjection(options: {
  agentId: string
  broadcast: (message: unknown) => void
  debug: (label: string, payload: unknown) => void
}) {
  let projectedThreadId: string | null = null
  let nextProjectedTurnId = 1
  const projectedItems = new Map<string, ProjectedItem>()

  const turnPayload = (turnId: string, status: "inProgress" | "completed") => ({
    id: turnId,
    items: [],
    itemsView: "notLoaded",
    status,
    error: null,
    startedAt: Math.floor(Date.now() / 1000),
    completedAt: status === "completed" ? Math.floor(Date.now() / 1000) : null,
    durationMs: null,
  })

  const startProjectedTurn = () => {
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
        turn: turnPayload(turnId, "inProgress"),
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
          completedAtMs: Date.now(),
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
          turn: turnPayload(projection.turnId, "completed"),
        },
      })
      projectedItems.delete(projection.key)
    }, 750)
  }

  const project = (records: TerminalOutputRecord[]) => {
    for (const record of records) {
      if (!projectedThreadId) continue
      if (record.agent_id !== options.agentId) continue
      const delta = Buffer.from(record.bytes).toString("utf8")
      if (!delta) continue
      const recordProjection = terminalRecordTranscriptProjection(record, delta, {
        isProviderIdleStatus,
        shouldRenderProviderStatus: () => false,
      })
      if (!recordProjection.appendsLiveTranscript) continue

      if (recordProjection.transcriptRole === "user") {
        const turnId = startProjectedTurn()
        if (!turnId) continue
        const itemId = `arroba-projected-user-${Date.now()}-${nextProjectedTurnId}`
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
            startedAtMs: Date.now(),
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
            completedAtMs: Date.now(),
          },
        })
        options.debug("projected_output_to_tui", { agentId: options.agentId, kind: record.kind, byteLength: record.bytes.length })
        continue
      }

      const itemKind = codexProjectedItemKind(recordProjection)
      if (!itemKind) continue
      const itemKey = `${itemKind}:${recordProjection.mergeKey ?? "default"}`
      let itemProjection = projectedItems.get(itemKey)
      if (!itemProjection) {
        const turnId = startProjectedTurn()
        if (!turnId) continue
        const itemId = `arroba-projected-${itemKind}-${Date.now()}-${nextProjectedTurnId}`
        itemProjection = { key: itemKey, turnId, itemId, kind: itemKind, text: "", timer: null }
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
            startedAtMs: Date.now(),
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

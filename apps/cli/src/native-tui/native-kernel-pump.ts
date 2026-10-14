import { traceCodexLifecycle } from "./codex-lifecycle-trace.js"
import type { AgentRuntimeActivity, RuntimeInteraction, TerminalOutputRecord } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  getSessionStateRequest,
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
} from "../ipc-requests.js"

export function startNativeKernelPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  options: {
    onAgentActivity?: ((activity: Record<string, AgentRuntimeActivity>) => void | Promise<void>) | undefined
    onInteractions?: ((interactions: RuntimeInteraction[]) => void) | undefined
    onTerminalRecords?: ((records: TerminalOutputRecord[]) => void) | undefined
    debug?: ((label: string, payload: unknown) => void) | undefined
    formatError?: ((error: unknown) => string) | undefined
    intervalMs?: number | undefined
    pollRuntimeNotices?: boolean | undefined
  } = {},
): { stop: () => void } {
  let stopped = false
  let inFlight = false
  const tick = async () => {
    if (stopped || inFlight) return
    inFlight = true
    try {
      // Read settlement before draining output: all final records admitted by
      // this snapshot must reach the TUI before its turn/completed notification.
      const state = options.onAgentActivity || options.onInteractions
        ? await client.send<{ SessionState?: { agent_activity?: Record<string, AgentRuntimeActivity>, session?: { active_interactions?: RuntimeInteraction[] } } }>(getSessionStateRequest(sessionId))
        : undefined
      if (stopped) return
      let outputDrained = false
      // TerminalOutput is size bounded. Keep each tick bounded too, and defer
      // settlement to a later tick if the attachment still has output pages.
      const maxPages = options.onAgentActivity ? 8 : 1
      for (let page = 0; !stopped && page < maxPages; page++) {
        const response = await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
        traceCodexLifecycle("pump_reply", { responseKinds: Object.keys(response),
          records: Array.isArray((response.TerminalOutput as { records?: unknown[] } | undefined)?.records)
            ? (response.TerminalOutput as { records: unknown[] }).records.length : undefined })
        if ("TerminalOutput" in response) {
          const records = (response.TerminalOutput as { records?: unknown[] }).records
          if (Array.isArray(records) && records.length === 0) {
            outputDrained = true
            break
          }
          if (!stopped && Array.isArray(records) && records.length > 0) {
            const terminalRecords = records as TerminalOutputRecord[]
            options.debug?.("pump_terminal_records", {
              sessionId,
              attachmentId,
              count: terminalRecords.length,
              records: terminalRecords.map(summarizeTerminalRecord),
            })
            options.onTerminalRecords?.(terminalRecords)
          }
        }
      }
      traceCodexLifecycle("pump_tick", { outputDrained })
      if (!stopped && outputDrained && state?.SessionState?.agent_activity) await options.onAgentActivity?.(state.SessionState.agent_activity)
      // Permission projections establish their own prompt-scoped display
      // barrier. Another agent's output must not hold their approval hostage.
      if (!stopped && state?.SessionState?.session) options.onInteractions?.(state.SessionState.session.active_interactions ?? [])
      if (!stopped && options.pollRuntimeNotices !== false) {
        await client.send<Record<string, unknown>>(pollRuntimeNoticesRequest(sessionId, attachmentId))
      }
    } catch (error) {
      traceCodexLifecycle("pump_error", { errorClass: error instanceof Error ? error.name : typeof error })
      options.debug?.("pump_error", {
        error: options.formatError ? options.formatError(error) : error instanceof Error ? error.message : String(error),
      })
    } finally {
      inFlight = false
    }
  }
  const intervalMs = options.intervalMs ?? 250
  const interval = setInterval(() => {
    void tick()
  }, intervalMs)
  void tick()
  return {
    stop: () => {
      stopped = true
      clearInterval(interval)
    },
  }
}

function summarizeTerminalRecord(record: TerminalOutputRecord) {
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

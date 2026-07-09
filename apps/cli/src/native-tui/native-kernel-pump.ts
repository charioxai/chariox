import type { TerminalOutputRecord } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  pollRuntimeNoticesRequest,
  pumpTerminalOutputRequest,
} from "../ipc-requests.js"

export function startNativeKernelPumpLoop(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  options: {
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
      if (options.onTerminalRecords) {
        const response = await client.send<Record<string, unknown>>(pumpTerminalOutputRequest(sessionId, attachmentId))
        if ("TerminalOutput" in response) {
          const records = (response.TerminalOutput as { records?: unknown[] }).records
          if (Array.isArray(records) && records.length > 0) {
            const terminalRecords = records as TerminalOutputRecord[]
            options.debug?.("pump_terminal_records", {
              sessionId,
              attachmentId,
              count: terminalRecords.length,
              records: terminalRecords.map(summarizeTerminalRecord),
            })
            options.onTerminalRecords(terminalRecords)
          }
        }
      }
      if (options.pollRuntimeNotices !== false) {
        await client.send<Record<string, unknown>>(pollRuntimeNoticesRequest(sessionId, attachmentId))
      }
    } catch (error) {
      options.debug?.("pump_error", {
        error: options.formatError ? options.formatError(error) : error instanceof Error ? error.message : String(error),
      })
    } finally {
      inFlight = false
    }
  }
  const interval = setInterval(() => {
    void tick()
  }, options.intervalMs ?? 250)
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

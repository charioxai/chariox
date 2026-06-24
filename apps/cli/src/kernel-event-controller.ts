import type { RuntimeNoticeRecord, TerminalOutputRecord, TranscriptEntry } from "./cli-types.js"
import { isProviderIdleStatus } from "./runtime.js"

type KernelEventControllerDeps = {
  recordDaemonActivity: (source: string) => void
  recordTurnActivity: (source: string) => void
  resolveTerminalRecordAgentId: (record: TerminalOutputRecord) => string | null
  setStreamingAgentId: (agentId: string) => void
  markAgentBusy: (agentId: string | null | undefined) => void
  splitAgentResponseMode: () => boolean
  visibleTranscriptAgentId: () => string | null
  focusedAgentId: () => string | null
  hasTrailingUserPrompt: (agentId: string, text: string, promptId?: string | null) => boolean
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  computeNextTurnId: (entries: TranscriptEntry[]) => number
  appendTranscriptEntryToAgentPane: (agentId: string, entry: Omit<TranscriptEntry, "id">) => void
  appendProviderChunkToAgentPane: (
    agentId: string,
    role: TranscriptEntry["role"],
    text: string,
    mergeKey?: string,
    sourceText?: string,
    metadata?: TerminalRecordTranscriptMetadata,
  ) => void
  appendToolUpdateToAgentPane: (agentId: string, text: string, metadata?: TerminalRecordTranscriptMetadata) => void
  setAgentActivityLabel: (agentId: string | null | undefined, label: string | null) => void
  agentActivityLabel: (agentId: string | null | undefined) => string | null
  setProviderActivityLabel: (label: string | null) => void
  applyProviderActivity: (active: boolean) => void
  syncVisibleActivityLabel: () => void
  getProviderActivityLabel: (text: string) => string | null
  shouldRenderProviderStatus: (text: string) => boolean
  appendEntry: (entry: Omit<TranscriptEntry, "id">) => void
  appendProviderChunk: (
    role: TranscriptEntry["role"],
    text: string,
    mergeKey?: string,
    sourceText?: string,
    metadata?: TerminalRecordTranscriptMetadata,
  ) => void
  appendToolUpdate: (text: string, metadata?: TerminalRecordTranscriptMetadata) => void
  appendProviderError: (text: string) => void
  syncVisibleTranscriptPreview: () => void
  appendAgentPanePreview: (agentId: string | null | undefined, line: string) => void
  previewLineForTerminalRecord: (kind: TerminalOutputRecord["kind"], text: string) => string
  trimSingleTrailingNewline: (text: string) => string
  setDaemonDisconnected: (value: boolean) => void
  setStatusLine: (value: string) => void
  updateSessionChrome: () => void
  appendNotice: (message: string, tone?: "default" | "warning") => void
  connectedStatusLine: string
  markAssistantMessageCompleted: (agentId: string | null | undefined) => void
  handleExternalProviderHistoryUpdated?: (agentId: string | null) => void
}

const EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS = "external_provider_history_updated"

type TerminalRecordTranscriptMetadata = {
  promptId?: string | null
  sourceAttachmentId?: string | null
}

export function createKernelEventController(deps: KernelEventControllerDeps) {
  let lastTransportNoticeMessage: string | null = null
  let lastTransportNoticeAtMs = 0

  const processTerminalOutputRecord = (record: TerminalOutputRecord) => {
    deps.recordDaemonActivity("terminal_record")
    deps.recordTurnActivity("terminal_record")
    const text = Buffer.from(record.bytes).toString("utf8")
    const recordAgentId = deps.resolveTerminalRecordAgentId(record)
    if (!recordAgentId) {
      return
    }
    if (record.kind === "provider_status" && text.trim() === EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS) {
      deps.handleExternalProviderHistoryUpdated?.(recordAgentId)
      return
    }
    if (record.kind !== "prompt_echo") {
      deps.setStreamingAgentId(recordAgentId)
      if (record.kind !== "provider_status" || !isProviderIdleStatus(text)) {
        deps.markAgentBusy(recordAgentId)
      }
    }
    if (deps.splitAgentResponseMode() && recordAgentId) {
      const metadata = terminalRecordTranscriptMetadata(record)
      if (record.kind === "provider_status") {
        if (isProviderIdleStatus(text)) {
          return
        }
        const activityLabel = deps.getProviderActivityLabel(text)
        deps.setAgentActivityLabel(recordAgentId, activityLabel)
        if (recordAgentId === deps.focusedAgentId()) {
          const nextFocusedActivityLabel = activityLabel ?? deps.agentActivityLabel(recordAgentId)
          deps.setProviderActivityLabel(nextFocusedActivityLabel)
          deps.applyProviderActivity(nextFocusedActivityLabel !== null)
          if (activityLabel !== null) {
            deps.syncVisibleActivityLabel()
          }
        }
      }
      switch (record.kind) {
        case "prompt_echo": {
          if (deps.hasTrailingUserPrompt(recordAgentId, text, record.prompt_id ?? null)) {
            break
          }
          const paneEntries = deps.currentAgentPaneEntries(recordAgentId)
          deps.appendTranscriptEntryToAgentPane(recordAgentId, {
            role: "user",
            text: deps.trimSingleTrailingNewline(text),
            turnId: deps.computeNextTurnId(paneEntries),
            ...metadata,
          })
          break
        }
        case "provider_reasoning":
          deps.appendProviderChunkToAgentPane(recordAgentId, "reasoning", text, record.merge_key, undefined, metadata)
          break
        case "provider_tool":
          deps.appendToolUpdateToAgentPane(recordAgentId, text, metadata)
          break
        case "provider_error": {
          const normalized = normalize(text).trim()
          if (normalized) {
            deps.appendTranscriptEntryToAgentPane(recordAgentId, { role: "error", text: normalized, emphasis: "error", ...metadata })
          }
          break
        }
        case "provider_status":
          if (deps.shouldRenderProviderStatus(text)) {
            deps.appendProviderChunkToAgentPane(recordAgentId, "status", text, "__provider_status__", undefined, metadata)
          }
          break
        default:
          deps.appendProviderChunkToAgentPane(recordAgentId, "assistant", text, record.merge_key, undefined, metadata)
          break
      }
      return
    }

    const mainTranscriptAgentId = deps.visibleTranscriptAgentId()
    const isVisibleRecord = recordAgentId === mainTranscriptAgentId
    if (!isVisibleRecord) {
      if (recordAgentId) {
        const metadata = terminalRecordTranscriptMetadata(record)
        switch (record.kind) {
          case "prompt_echo": {
            if (deps.hasTrailingUserPrompt(recordAgentId, text, record.prompt_id ?? null)) {
              break
            }
            const paneEntries = deps.currentAgentPaneEntries(recordAgentId)
            deps.appendTranscriptEntryToAgentPane(recordAgentId, {
              role: "user",
              text: deps.trimSingleTrailingNewline(text),
              turnId: deps.computeNextTurnId(paneEntries),
              ...metadata,
            })
            break
          }
          case "provider_reasoning":
            deps.appendProviderChunkToAgentPane(recordAgentId, "reasoning", text, record.merge_key, undefined, metadata)
            break
          case "provider_tool":
            deps.appendToolUpdateToAgentPane(recordAgentId, text, metadata)
            break
          case "provider_error": {
            const normalized = normalize(text).trim()
            if (normalized) {
              deps.appendTranscriptEntryToAgentPane(recordAgentId, { role: "error", text: normalized, emphasis: "error", ...metadata })
            }
            break
          }
          case "provider_status":
            if (isProviderIdleStatus(text)) {
              break
            }
            deps.setAgentActivityLabel(recordAgentId, deps.getProviderActivityLabel(text))
            if (deps.shouldRenderProviderStatus(text)) {
              deps.appendProviderChunkToAgentPane(recordAgentId, "status", text, "__provider_status__", undefined, metadata)
            }
            break
          default:
            deps.appendProviderChunkToAgentPane(recordAgentId, "assistant", text, record.merge_key, undefined, metadata)
            break
        }
      }
      deps.appendAgentPanePreview(recordAgentId, deps.previewLineForTerminalRecord(record.kind, text))
      return
    }

    const metadata = terminalRecordTranscriptMetadata(record)
    switch (record.kind) {
      case "prompt_echo":
        if (recordAgentId && deps.hasTrailingUserPrompt(recordAgentId, text, record.prompt_id ?? null)) {
          break
        }
        deps.appendEntry({ role: "user", text: deps.trimSingleTrailingNewline(text), ...metadata })
        deps.syncVisibleTranscriptPreview()
        break
      case "provider_reasoning":
        deps.appendProviderChunk("reasoning", text, record.merge_key, undefined, metadata)
        deps.syncVisibleTranscriptPreview()
        break
      case "provider_tool":
        deps.appendToolUpdate(text, metadata)
        deps.syncVisibleTranscriptPreview()
        break
      case "provider_error":
        deps.appendProviderError(text)
        deps.syncVisibleTranscriptPreview()
        break
      case "provider_status": {
        if (isProviderIdleStatus(text)) {
          break
        }
        const activityLabel = deps.getProviderActivityLabel(text)
        deps.setAgentActivityLabel(recordAgentId, activityLabel)
        const nextFocusedActivityLabel = activityLabel ?? deps.agentActivityLabel(recordAgentId)
        deps.setProviderActivityLabel(nextFocusedActivityLabel)
        deps.applyProviderActivity(nextFocusedActivityLabel !== null)
        if (activityLabel !== null) {
          deps.syncVisibleActivityLabel()
        }
        if (deps.shouldRenderProviderStatus(text)) {
          deps.appendProviderChunk("status", text, "__provider_status__", undefined, metadata)
          deps.syncVisibleTranscriptPreview()
        }
        break
      }
      default:
        deps.appendProviderChunk("assistant", text, record.merge_key, undefined, metadata)
        deps.syncVisibleTranscriptPreview()
        break
    }
  }

  const applyRuntimeNotices = (notices: RuntimeNoticeRecord[]) => {
    deps.recordDaemonActivity("kernel_runtime_notices")
    for (const notice of notices) {
      deps.appendNotice(notice.message)
    }
  }

  const applyTransportClosed = (message: string) => {
    deps.setDaemonDisconnected(true)
    deps.setStatusLine("Lost connection to the Arroba kernel.")
    deps.updateSessionChrome()
    const now = Date.now()
    if (message !== lastTransportNoticeMessage || now - lastTransportNoticeAtMs > 10_000) {
      lastTransportNoticeMessage = message
      lastTransportNoticeAtMs = now
      deps.appendNotice(message, "warning")
    }
  }

  const applyTransportResumed = () => {
    deps.recordDaemonActivity("kernel_transport_resumed")
    deps.setDaemonDisconnected(false)
    deps.setStatusLine(deps.connectedStatusLine)
    deps.updateSessionChrome()
    if (lastTransportNoticeMessage !== null) {
      deps.appendNotice("Reconnected to the Arroba kernel.")
    }
    lastTransportNoticeMessage = null
    lastTransportNoticeAtMs = 0
  }

  const applyAssistantMessageCompleted = (event: {
    agent_id?: string | null
  }) => {
    deps.recordDaemonActivity("assistant_message_completed")
    deps.markAssistantMessageCompleted(event.agent_id ?? null)
  }

  return {
    processTerminalOutputRecord,
    applyRuntimeNotices,
    applyTransportClosed,
    applyTransportResumed,
    applyAssistantMessageCompleted,
  }
}

function normalize(text: string) {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
}

function terminalRecordTranscriptMetadata(record: TerminalOutputRecord): TerminalRecordTranscriptMetadata {
  return {
    ...(record.prompt_id !== undefined ? { promptId: record.prompt_id } : {}),
    ...(record.source_attachment_id !== undefined ? { sourceAttachmentId: record.source_attachment_id } : {}),
  }
}

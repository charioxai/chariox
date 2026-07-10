import type { RuntimeNoticeRecord, TerminalOutputRecord, TranscriptEntry } from "./cli-types.js"
import {
  terminalRecordTranscriptProjection,
  transcriptEntryWithTerminalMetadata,
  type TerminalRecordTranscriptMetadata,
  type TerminalRecordTranscriptProjection,
} from "@arroba/kernel-client/terminal-record-transcript"
import { createTranscriptSteeredPromptEntry } from "@arroba/kernel-client/transcript-entry-state"
import { isProviderIdleStatus } from "@arroba/kernel-client/provider-status"
import { runtimeNoticeShouldRenderInAgentPane } from "./runtime-notice-filter.js"

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

type ProjectedRecordAppendTarget = {
  user: (text: string, projection: TerminalRecordTranscriptProjection) => void
  reasoning: (text: string, projection: TerminalRecordTranscriptProjection) => void
  tool: (text: string, metadata: TerminalRecordTranscriptMetadata) => void
  error: (text: string, metadata: TerminalRecordTranscriptMetadata) => void
  status: (text: string, projection: TerminalRecordTranscriptProjection) => void
  assistant: (text: string, projection: TerminalRecordTranscriptProjection) => void
}

export function createKernelEventController(deps: KernelEventControllerDeps) {
  let lastTransportNoticeMessage: string | null = null
  let lastTransportNoticeAtMs = 0

  const appendProjectedRecord = (
    projection: TerminalRecordTranscriptProjection,
    target: ProjectedRecordAppendTarget,
  ) => {
    const metadata = projection.metadata
    switch (projection.transcriptRole) {
      case "user": {
        target.user(projection.transcriptText, projection)
        break
      }
      case "reasoning":
        target.reasoning(projection.transcriptText, projection)
        break
      case "tool":
        target.tool(projection.transcriptText, metadata)
        break
      case "error":
        target.error(projection.transcriptText, metadata)
        break
      case "status":
        if (projection.renderProviderStatus) {
          target.status(projection.transcriptText, projection)
        }
        break
      case "assistant":
        target.assistant(projection.transcriptText, projection)
        break
      case null:
        break
    }
  }

  const appendProjectedRecordToAgentPane = (
    recordAgentId: string,
    projection: TerminalRecordTranscriptProjection,
  ) => appendProjectedRecord(projection, {
    user: (text, item) => {
      const metadata = item.metadata
      if (deps.hasTrailingUserPrompt(recordAgentId, text, metadata.promptId ?? null)) {
        return
      }
      if (item.steeringPrompt) {
        const entry = createTranscriptSteeredPromptEntry(text, metadata)
        if (entry) deps.appendTranscriptEntryToAgentPane(recordAgentId, entry)
        return
      }
      const paneEntries = deps.currentAgentPaneEntries(recordAgentId)
      deps.appendTranscriptEntryToAgentPane(recordAgentId, transcriptEntryWithTerminalMetadata<Omit<TranscriptEntry, "id">>({
        role: "user",
        text: deps.trimSingleTrailingNewline(text),
        turnId: deps.computeNextTurnId(paneEntries),
      }, metadata))
    },
    reasoning: (text, item) => {
      deps.appendProviderChunkToAgentPane(recordAgentId, "reasoning", text, item.mergeKey ?? undefined, undefined, item.metadata)
    },
    tool: (text, metadata) => {
      deps.appendToolUpdateToAgentPane(recordAgentId, text, metadata)
    },
    error: (text, metadata) => {
      if (text) {
        deps.appendTranscriptEntryToAgentPane(recordAgentId, transcriptEntryWithTerminalMetadata<Omit<TranscriptEntry, "id">>({ role: "error", text, emphasis: "error" }, metadata))
      }
    },
    status: (text, item) => {
      deps.appendProviderChunkToAgentPane(recordAgentId, "status", text, item.statusMergeKey ?? undefined, undefined, item.metadata)
    },
    assistant: (text, item) => {
      deps.appendProviderChunkToAgentPane(recordAgentId, "assistant", text, item.mergeKey ?? undefined, undefined, item.metadata)
    },
  })

  const appendProjectedRecordToVisibleTranscript = (
    recordAgentId: string,
    projection: TerminalRecordTranscriptProjection,
  ) => appendProjectedRecord(projection, {
    user: (text, item) => {
      const metadata = item.metadata
      if (deps.hasTrailingUserPrompt(recordAgentId, text, metadata.promptId ?? null)) {
        return
      }
      if (item.steeringPrompt) {
        const entry = createTranscriptSteeredPromptEntry(text, metadata)
        if (entry) deps.appendEntry(entry)
        deps.syncVisibleTranscriptPreview()
        return
      }
      deps.appendEntry(transcriptEntryWithTerminalMetadata<Omit<TranscriptEntry, "id">>({ role: "user", text: deps.trimSingleTrailingNewline(text) }, metadata))
      deps.syncVisibleTranscriptPreview()
    },
    reasoning: (text, item) => {
      deps.appendProviderChunk("reasoning", text, item.mergeKey ?? undefined, undefined, item.metadata)
      deps.syncVisibleTranscriptPreview()
    },
    tool: (text, metadata) => {
      deps.appendToolUpdate(text, metadata)
      deps.syncVisibleTranscriptPreview()
    },
    error: (text) => {
      deps.appendProviderError(text)
      deps.syncVisibleTranscriptPreview()
    },
    status: (text, item) => {
      deps.appendProviderChunk("status", text, item.statusMergeKey ?? undefined, undefined, item.metadata)
      deps.syncVisibleTranscriptPreview()
    },
    assistant: (text, item) => {
      deps.appendProviderChunk("assistant", text, item.mergeKey ?? undefined, undefined, item.metadata)
      deps.syncVisibleTranscriptPreview()
    },
  })

  const processTerminalOutputRecord = (record: TerminalOutputRecord) => {
    deps.recordDaemonActivity("terminal_record")
    deps.recordTurnActivity("terminal_record")
    const text = Buffer.from(record.bytes).toString("utf8")
    const recordAgentId = deps.resolveTerminalRecordAgentId(record)
    if (!recordAgentId) {
      return
    }
    const projection = terminalRecordTranscriptProjection(record, text, {
      isProviderIdleStatus,
      shouldRenderProviderStatus: deps.shouldRenderProviderStatus,
    })
    if (projection.historyRefreshSignal) {
      deps.handleExternalProviderHistoryUpdated?.(recordAgentId)
      return
    }
    if (projection.passiveExternalTelemetry) {
      return
    }
    if (projection.startsStreaming) {
      deps.setStreamingAgentId(recordAgentId)
      if (projection.marksAgentBusy) {
        deps.markAgentBusy(recordAgentId)
      }
    }
    if (deps.splitAgentResponseMode() && recordAgentId) {
      if (projection.updatesProviderActivity) {
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
      if (!projection.appendsLiveTranscript) {
        return
      }
      appendProjectedRecordToAgentPane(recordAgentId, projection)
      return
    }

    const mainTranscriptAgentId = deps.visibleTranscriptAgentId()
    const isVisibleRecord = recordAgentId === mainTranscriptAgentId
    if (!isVisibleRecord) {
      if (recordAgentId) {
        if (projection.updatesProviderActivity) {
          deps.setAgentActivityLabel(recordAgentId, deps.getProviderActivityLabel(text))
        }
        if (projection.appendsLiveTranscript) {
          appendProjectedRecordToAgentPane(recordAgentId, projection)
        }
      }
      deps.appendAgentPanePreview(recordAgentId, deps.previewLineForTerminalRecord(record.kind, text))
      return
    }

    if (projection.updatesProviderActivity) {
      const activityLabel = deps.getProviderActivityLabel(text)
      deps.setAgentActivityLabel(recordAgentId, activityLabel)
      const nextFocusedActivityLabel = activityLabel ?? deps.agentActivityLabel(recordAgentId)
      deps.setProviderActivityLabel(nextFocusedActivityLabel)
      deps.applyProviderActivity(nextFocusedActivityLabel !== null)
      if (activityLabel !== null) {
        deps.syncVisibleActivityLabel()
      }
    }
    if (!projection.appendsLiveTranscript) {
      return
    }
    appendProjectedRecordToVisibleTranscript(recordAgentId, projection)
  }

  const applyRuntimeNotices = (notices: RuntimeNoticeRecord[]) => {
    deps.recordDaemonActivity("kernel_runtime_notices")
    for (const notice of notices) {
      if (!runtimeNoticeShouldRenderInAgentPane(notice.message)) {
        continue
      }
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

import { createAssistantMessageCompletionController } from "./assistant-message-completion-controller.js"
import { createAuthoritativeIdleController } from "./authoritative-idle-controller.js"
import {
  LIVE_TRANSCRIPT_LIMIT,
  LIVE_TRANSCRIPT_MAX_CHARS,
  STREAM_BATCH_WINDOW_MS,
  STREAM_RECORDS_PER_FLUSH,
  TURN_COMPLETION_QUIET_MS,
} from "./cli-runtime-tuning.js"
import type { TerminalOutputRecord } from "./cli-types.js"
import { createProviderActivityController } from "./provider-activity-controller.js"
import { getTurnCompletionDelayMs } from "./runtime.js"
import { getSessionHistoryBlobContent } from "./session-history-api.js"
import { sessionHasPromptWork } from "./session-state.js"
import { createTerminalOutputRecordProcessor } from "./terminal-output-record-processor.js"
import { createTerminalOutputRecordQueue } from "./terminal-output-record-queue.js"
import { createTranscriptEventController } from "./transcript-event-controller.js"
import { createTranscriptRetentionController } from "./transcript-retention-controller.js"
import { createTranscriptStateController } from "./transcript-state-controller.js"
import { createTranscriptStreamController } from "./transcript-stream-controller.js"
import { createTranscriptTurnExpansionController } from "./transcript-turn-expansion-controller.js"
import { createTranscriptViewportController } from "./transcript-viewport-controller.js"
import { createTurnCompletionController } from "./turn-completion-controller.js"
import { createVisibleActivityLabelController } from "./visible-activity-label-controller.js"

type AnyFn = (...args: any[]) => any

export type CliTranscriptRuntimeCompositionDeps = {
  batchUpdate: AnyFn
  client: any
  formatError: AnyFn
  scheduleTimer: AnyFn
  clearTimer: AnyFn
  runUiBatch: AnyFn
  entries: AnyFn
  setEntries: AnyFn
  entryCounter: AnyFn
  setEntryCounter: AnyFn
  sessionState: AnyFn
  activePrompt: AnyFn
  statusLine: AnyFn
  setStatusLine: AnyFn
  setWorking: AnyFn
  setSubmitting: AnyFn
  setStreamingAgentId: AnyFn
  setAgentActivityLabels: AnyFn
  setAgentBusyLatches: AnyFn
  setProviderActivityLabel: AnyFn
  setActiveStatusLabel: AnyFn
  promptSubmissionAgentStateController: {
    setSubmittingAgentId: AnyFn
    clearSubmittingAgentId: AnyFn
  }
  promptStopController: {
    reset: AnyFn
  }
  appendPromptEchoToSharedHistory: AnyFn
  focusedAgentId: AnyFn
  visibleTranscriptAgentId: AnyFn
  responsePrimaryAgent: AnyFn
  splitAgentResponseMode: AnyFn
  isAttached: AnyFn
  currentAgentPaneEntries: AnyFn
  appendTranscriptEntryToAgentPane: AnyFn
  transcriptEntryProjectionController: {
    renderableEntries: AnyFn
  }
  transcriptTurnStateController: {
    getCurrentTurnId: AnyFn
    getNextTurnId: AnyFn
    setNextTurnId: AnyFn
    setCurrentTurnId: AnyFn
  }
  expandedTurnIdsByAgent: AnyFn
  setExpandedTurnIdsByAgent: AnyFn
  persistVisibleTranscriptEntries: AnyFn
  reconcileMountedTranscript: AnyFn
  retainPromptFocus: AnyFn
  transcriptScrollboxRefController: {
    current: AnyFn
    remove: AnyFn
    requestRender: AnyFn
  }
  historyScrollRestoreController: {
    cancel: AnyFn
  }
  primaryTranscriptRuntimeStore: {
    transcriptRenderables: any
    tools: any
    activeToolLabels: any
    clearActiveToolLabels: AnyFn
    deleteTool: AnyFn
    setLastScrollTop: AnyFn
  }
  clearAgentBusy: AnyFn
  markAgentBusy: AnyFn
  setWaitingRoomCloudNotice: AnyFn
  renderSessionChromeBoundary: AnyFn
  syncVisibleTranscriptPreview: AnyFn
  updateSessionChrome: AnyFn
  rebuildTranscript: AnyFn
  focusedActivityLabel: AnyFn
  logVisibleTranscriptOutput: AnyFn
  updateTranscriptEntry: AnyFn
  setAgentTranscriptEntries: AnyFn
  DEFAULT_CONNECTED_STATUS: string
}

export function createCliTranscriptRuntimeComposition(deps: CliTranscriptRuntimeCompositionDeps) {
  let processKernelTerminalOutputRecord: (record: TerminalOutputRecord) => void = () => {}
  const terminalOutputRecordProcessor = createTerminalOutputRecordProcessor({
    appendPromptEchoToSharedHistory: deps.appendPromptEchoToSharedHistory,
    processKernelTerminalOutputRecord: (record) => {
      processKernelTerminalOutputRecord(record)
    },
  })
  const terminalOutputRecordQueue = createTerminalOutputRecordQueue<ReturnType<typeof deps.scheduleTimer>, TerminalOutputRecord>({
    delayMs: STREAM_BATCH_WINDOW_MS,
    maxRecordsPerFlush: STREAM_RECORDS_PER_FLUSH,
    scheduleTimer: deps.scheduleTimer,
    clearTimer: deps.clearTimer,
    processRecords(records) {
      deps.runUiBatch(() => {
        for (const record of records) {
          terminalOutputRecordProcessor.process(record)
        }
      })
    },
  })

  const turnCompletionController = createTurnCompletionController<ReturnType<typeof deps.scheduleTimer>>({
    now: Date.now,
    scheduleTimer: deps.scheduleTimer,
    clearTimer: deps.clearTimer,
    hasActivePrompt: () => Boolean(deps.activePrompt()),
    getDelayMs: (lastTurnActivityAt) => getTurnCompletionDelayMs({
      sessionHasPromptWork: sessionHasPromptWork(deps.sessionState()),
      pendingTerminalRecordCount: terminalOutputRecordQueue.pendingCount(),
      pendingTerminalRecordFlush: terminalOutputRecordQueue.hasPendingFlush(),
      lastTurnActivityAt,
      now: Date.now(),
      quietWindowMs: TURN_COMPLETION_QUIET_MS,
    }),
    completeTurn: () => {
      deps.batchUpdate(() => {
        deps.primaryTranscriptRuntimeStore.clearActiveToolLabels()
        deps.setAgentActivityLabels({})
        deps.setStreamingAgentId(null)
        deps.setSubmitting(false)
        deps.promptSubmissionAgentStateController.clearSubmittingAgentId()
        deps.setAgentBusyLatches({})
        deps.setProviderActivityLabel(null)
        deps.setActiveStatusLabel(null)
        if (!deps.activePrompt() && deps.statusLine() === "Cancellation requested.") {
          deps.setStatusLine(deps.DEFAULT_CONNECTED_STATUS)
        }
        deps.setWorking(false)
      })
      deps.renderSessionChromeBoundary()
    },
  })
  const cancelPendingTurnCompletion = turnCompletionController.cancelPending
  const recordTurnActivity = (_activityType: string) => {
    turnCompletionController.recordActivity()
  }
  const maybeScheduleConfirmedTurnCompletion = turnCompletionController.maybeScheduleConfirmed

  const expandedTurnIdsForAgent = (agentId: string | null | undefined) =>
    agentId ? (deps.expandedTurnIdsByAgent()[agentId] ?? []) : []
  const transcriptTurnExpansionController = createTranscriptTurnExpansionController({
    expandedTurnIdsForAgent,
    updateExpandedTurnIdsByAgent: (updater) => {
      deps.setExpandedTurnIdsByAgent((current: any) => updater(current))
    },
  })
  const setExpandedTurnState = transcriptTurnExpansionController.setExpandedTurnState
  const applyExpandedTurns = transcriptTurnExpansionController.applyExpandedTurns

  const transcriptRetentionController = createTranscriptRetentionController({
    entries: () => deps.entries().slice(),
    setEntries: deps.setEntries,
    renderables: deps.primaryTranscriptRuntimeStore.transcriptRenderables,
    removeFromScrollbox: (renderableId) => {
      return deps.transcriptScrollboxRefController.remove(renderableId)
    },
    requestScrollboxRender: deps.transcriptScrollboxRefController.requestRender,
    deleteTool: deps.primaryTranscriptRuntimeStore.deleteTool,
    maxEntries: LIVE_TRANSCRIPT_LIMIT,
    maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
  })
  const enforceTranscriptRetention = transcriptRetentionController.enforce

  const transcriptStateController = createTranscriptStateController({
    entries: deps.transcriptEntryProjectionController.renderableEntries,
    setEntries: deps.setEntries,
    entryCounter: deps.entryCounter,
    setEntryCounter: deps.setEntryCounter,
    currentTurnId: deps.transcriptTurnStateController.getCurrentTurnId,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    expandedTurnIdsForAgent,
    setExpandedTurnState: (agentId, turnId, expanded) => {
      setExpandedTurnState(agentId, turnId, expanded)
    },
    persistVisibleTranscriptEntries: deps.persistVisibleTranscriptEntries,
    reconcileMountedTranscript: deps.reconcileMountedTranscript,
    retainPromptFocus: deps.retainPromptFocus,
    enforceTranscriptRetention,
    loadHistoryBlobContent: (agentId, blobId) => getSessionHistoryBlobContent(
      deps.client,
      deps.sessionState().id,
      agentId,
      blobId,
    ),
    formatError: deps.formatError,
  })
  const applyVisibleTranscriptState = transcriptStateController.applyVisibleState
  const toggleTurn = transcriptStateController.toggleTurn
  const toggleBlob = transcriptStateController.toggleBlob
  const appendEntry = transcriptStateController.appendEntry

  const transcriptViewportController = createTranscriptViewportController({
    getScrollbox: deps.transcriptScrollboxRefController.current,
    cancelHistoryScrollRestore: () => deps.historyScrollRestoreController.cancel(),
    setLastTranscriptScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
  })

  const transcriptEventController = createTranscriptEventController({
    recordTurnActivity,
    resetTurnCompletion: () => turnCompletionController.reset(),
    cancelPendingTurnCompletion,
    focusedAgentId: deps.focusedAgentId,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    responsePrimaryAgent: deps.responsePrimaryAgent,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    isAttached: deps.isAttached,
    entries: deps.entries,
    nextTurnId: deps.transcriptTurnStateController.getNextTurnId,
    setNextTurnId: deps.transcriptTurnStateController.setNextTurnId,
    setCurrentTurnId: deps.transcriptTurnStateController.setCurrentTurnId,
    setSubmittingAgentId: deps.promptSubmissionAgentStateController.setSubmittingAgentId,
    setStreamingAgentId: deps.setStreamingAgentId,
    markAgentBusy: deps.markAgentBusy,
    clearAgentBusy: deps.clearAgentBusy,
    currentAgentPaneEntries: deps.currentAgentPaneEntries,
    collapseLatestTurnForAgent: (agentId, paneEntries) =>
      transcriptTurnExpansionController.collapseLatestTurnForAgent(agentId, paneEntries),
    appendTranscriptEntryToAgentPane: (agentId, entry, turnIds) => {
      deps.appendTranscriptEntryToAgentPane(agentId, entry, turnIds ? [...turnIds] : undefined)
    },
    appendEntry,
    setSubmitting: deps.setSubmitting,
    setWorking: deps.setWorking,
    renderSessionChromeBoundary: deps.renderSessionChromeBoundary,
    syncVisibleTranscriptPreview: deps.syncVisibleTranscriptPreview,
    scrollTranscriptToBottom: transcriptViewportController.scrollToBottom,
    updateSessionChrome: deps.updateSessionChrome,
    setWaitingRoomCloudNotice: deps.setWaitingRoomCloudNotice,
    rebuildTranscript: deps.rebuildTranscript,
  })
  const appendUserPrompt = transcriptEventController.appendUserPrompt
  const appendSteeredPrompt = transcriptEventController.appendSteeredPrompt
  const appendNotice = transcriptEventController.appendNotice
  const appendCloudNotice = transcriptEventController.appendCloudNotice
  const appendProviderError = transcriptEventController.appendProviderError

  const authoritativeIdleController = createAuthoritativeIdleController({
    batchUpdate: deps.batchUpdate,
    resetTurnCompletion: turnCompletionController.reset,
    clearActiveToolLabels: deps.primaryTranscriptRuntimeStore.clearActiveToolLabels,
    setAgentActivityLabels: deps.setAgentActivityLabels,
    setStreamingAgentId: deps.setStreamingAgentId,
    setSubmitting: deps.setSubmitting,
    clearSubmittingAgentId: deps.promptSubmissionAgentStateController.clearSubmittingAgentId,
    resetPromptStop: deps.promptStopController.reset,
    setAgentBusyLatches: deps.setAgentBusyLatches,
    setProviderActivityLabel: deps.setProviderActivityLabel,
    setActiveStatusLabel: deps.setActiveStatusLabel,
    setWorking: deps.setWorking,
    getStatusLine: deps.statusLine,
    setStatusLine: deps.setStatusLine,
    renderSessionChromeBoundary: deps.renderSessionChromeBoundary,
  })

  const providerActivityController = createProviderActivityController({
    setWorking: deps.setWorking,
    handleProviderActivity: turnCompletionController.handleProviderActivity,
    updateSessionChrome: deps.updateSessionChrome,
  })

  const assistantMessageCompletionController = createAssistantMessageCompletionController({
    entries: deps.transcriptEntryProjectionController.renderableEntries,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    currentAgentPaneEntries: deps.currentAgentPaneEntries,
    expandedTurnIdsForAgent,
    setExpandedTurnIdsForAgent: (agentId, turnIds) => {
      deps.setExpandedTurnIdsByAgent((current: any) => ({
        ...current,
        [agentId]: turnIds,
      }))
    },
    setEntries: deps.setEntries,
    setEntryCounter: deps.setEntryCounter,
    persistVisibleTranscriptEntries: deps.persistVisibleTranscriptEntries,
    reconcileMountedTranscript: deps.reconcileMountedTranscript,
    setAgentTranscriptEntries: deps.setAgentTranscriptEntries,
    clearAgentBusy: deps.clearAgentBusy,
    confirmTurnCompletion: turnCompletionController.confirm,
    maybeScheduleConfirmedTurnCompletion,
  })

  const visibleActivityLabelController = createVisibleActivityLabelController({
    focusedActivityLabel: deps.focusedActivityLabel,
    setActiveStatusLabel: deps.setActiveStatusLabel,
  })
  const syncVisibleActivityLabel = visibleActivityLabelController.sync

  const transcriptStreamController = createTranscriptStreamController({
    entries: deps.entries,
    setEntries: deps.setEntries,
    entryCounter: deps.entryCounter,
    currentTurnId: deps.transcriptTurnStateController.getCurrentTurnId,
    tools: deps.primaryTranscriptRuntimeStore.tools,
    activeToolLabels: deps.primaryTranscriptRuntimeStore.activeToolLabels,
    cancelPendingTurnCompletion,
    setWorking: deps.setWorking,
    setSubmitting: deps.setSubmitting,
    updateSessionChrome: deps.updateSessionChrome,
    syncVisibleActivityLabel,
    applyVisibleTranscriptState,
    persistVisibleTranscriptEntries: deps.persistVisibleTranscriptEntries,
    reconcileMountedTranscript: deps.reconcileMountedTranscript,
    updateTranscriptEntry: deps.updateTranscriptEntry,
    logVisibleTranscriptOutput: deps.logVisibleTranscriptOutput,
    enforceTranscriptRetention,
    maybeScheduleConfirmedTurnCompletion,
  })

  return {
    turnCompletionController,
    cancelPendingTurnCompletion,
    recordTurnActivity,
    expandedTurnIdsForAgent,
    setExpandedTurnState,
    applyExpandedTurns,
    toggleTurn,
    toggleBlob,
    appendEntry,
    appendUserPrompt,
    appendSteeredPrompt,
    appendNotice,
    appendCloudNotice,
    appendProviderError,
    clearLocalBusyStateForAuthoritativeIdle: authoritativeIdleController.clear,
    applyProviderActivity: providerActivityController.apply,
    markAssistantMessageCompleted: assistantMessageCompletionController.markCompleted,
    syncVisibleActivityLabel,
    appendProviderChunk: transcriptStreamController.appendProviderChunk,
    appendToolUpdate: transcriptStreamController.appendToolUpdate,
    queueTerminalOutputRecords: terminalOutputRecordQueue.queue,
    clearTerminalOutputRecordTimer: terminalOutputRecordQueue.clearTimer,
    setKernelTerminalOutputRecordProcessor(processor: (record: TerminalOutputRecord) => void) {
      processKernelTerminalOutputRecord = processor
    },
  }
}

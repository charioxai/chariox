import { createEffect, createMemo } from "solid-js"

import { createAgentRuntimeProjectionController } from "./agent-runtime-projection-controller.js"
import {
  createCliRuntimeDebugLogger,
} from "./cli-runtime-debug-logger.js"
import { createResponsePaneProjectionController } from "./response-pane-projection-controller.js"
import {
  sessionActiveInteractionForAgent,
  sessionFocusedInteraction,
} from "@arroba/kernel-client/session-runtime-lookup"
import { sessionFocusedStatusBadge } from "@arroba/kernel-client/session-runtime-status"
import { sessionFocusedAgentId } from "@arroba/kernel-client/session-runtime-transition"
import { createTranscriptEntryProjectionController } from "./transcript-entry-projection-controller.js"
import {
  deriveWorkflowPromptState,
} from "@arroba/kernel-client/workflow-prompt-state"

type AnyFn = (...args: any[]) => any

export type CliRuntimeProjectionCompositionDeps = {
  appLogger: any
  debugLogsEnabled: boolean
  attachmentState: AnyFn
  sessionState: AnyFn
  workspaceScreenMode: AnyFn
  multiAgentResponseLayout: AnyFn
  maxAgentsPerScreen: AnyFn
  workflowScreenActive: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  providerRunState: AnyFn
  primaryTranscriptRuntimeStore: {
    activeToolLabelValues: AnyFn
  }
  agentPaneRuntimeStore: {
    toolUpdatesForAgent: AnyFn
  }
  agentPanePreviews: AnyFn
  agentActivityLabels: AnyFn
  setAgentActivityLabels: AnyFn
  agentBusyLatches: AnyFn
  setAgentBusyLatches: AnyFn
  submitting: AnyFn
  promptSubmissionAgentStateController: {
    getSubmittingAgentId: AnyFn
  }
  streamingAgentId: AnyFn
  entries: AnyFn
  daemonDisconnected: AnyFn
  transcriptScrollboxRefController: {
    hasScrollbox: AnyFn
  }
}

export function createCliRuntimeProjectionComposition(
  deps: CliRuntimeProjectionCompositionDeps,
) {
  const isAttached = () => deps.attachmentState() !== null
  const focusedAgentId = () => sessionFocusedAgentId(deps.sessionState())
  const responsePaneProjectionController = createResponsePaneProjectionController({
    isAttached,
    getSession: deps.sessionState,
    getFocusedAgentId: focusedAgentId,
    getWorkspaceScreenMode: deps.workspaceScreenMode,
    getResponseLayout: deps.multiAgentResponseLayout,
    getMaxAgentsPerScreen: deps.maxAgentsPerScreen,
    workflowScreenActive: () => deps.workflowScreenActive(),
  })
  const multiAgentMode = responsePaneProjectionController.multiAgentMode
  const workflowScreenShowing = responsePaneProjectionController.workflowScreenShowing
  const splitAgentResponseMode = responsePaneProjectionController.splitAgentResponseMode
  const activeInteractionForAgent = (agentId: string | null | undefined) =>
    sessionActiveInteractionForAgent(deps.sessionState(), agentId)
  const focusedAgentInteraction = () => sessionFocusedInteraction(deps.sessionState())
  const workflowPromptState = createMemo(() => deriveWorkflowPromptState({
    workflowScreenActive: workflowScreenShowing(),
    workflows: deps.sessionState().workflows ?? [],
    workflowRuns: deps.sessionState().workflow_runs ?? [],
    agents: deps.sessionState().agents ?? [],
    selectedWorkflowId: deps.selectedWorkflowId(),
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId(),
  }))
  const responsePaneSelection = responsePaneProjectionController.responsePaneSelection
  const responsePaneAgentSignature = responsePaneProjectionController.responsePaneAgentSignature
  const responsePrimaryAgent = responsePaneProjectionController.responsePrimaryAgent
  const responseVisibleAgents = responsePaneProjectionController.responseVisibleAgents
  const visibleTranscriptAgentId = responsePaneProjectionController.visibleTranscriptAgentId
  const responsePaneRows = responsePaneProjectionController.responsePaneRows
  const primaryTranscriptSurfaceTone = responsePaneProjectionController.primaryTranscriptSurfaceTone
  const auxiliaryTranscriptSurfaceTone = responsePaneProjectionController.auxiliaryTranscriptSurfaceTone
  const agentRuntimeProjectionController = createAgentRuntimeProjectionController({
    getSession: deps.sessionState,
    getFocusedAgentId: focusedAgentId,
    getProviderRun: deps.providerRunState,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
    getActiveToolLabels: deps.primaryTranscriptRuntimeStore.activeToolLabelValues,
    getAgentPaneToolUpdates: deps.agentPaneRuntimeStore.toolUpdatesForAgent,
    getAgentPanePreviews: deps.agentPanePreviews,
    getAgentActivityLabels: deps.agentActivityLabels,
    updateAgentActivityLabels: (updater) => {
      deps.setAgentActivityLabels((current: any) => updater(current))
    },
    getAgentBusyLatches: deps.agentBusyLatches,
    updateAgentBusyLatches: (updater) => {
      deps.setAgentBusyLatches((current: any) => updater(current))
    },
    getSubmitting: deps.submitting,
    getSubmittingAgentId: deps.promptSubmissionAgentStateController.getSubmittingAgentId,
    getStreamingAgentId: deps.streamingAgentId,
  })
  const transcriptEntryProjectionController = createTranscriptEntryProjectionController({
    getEntries: () => deps.entries(),
  })
  const visibleTranscriptEntries = transcriptEntryProjectionController.visibleEntries
  const connectedClientCount = () => deps.sessionState().attachment_ids.length
  const activePrompt = () => agentRuntimeProjectionController.focusedActivePrompt()
  const focusedStatusBadge = () => sessionFocusedStatusBadge({
    attached: isAttached(),
    daemonDisconnected: deps.daemonDisconnected(),
    activeStatusLabel: agentRuntimeProjectionController.focusedActivityLabel(),
    focusedBusy: agentRuntimeProjectionController.focusedAgentBusy(),
    agents: agentRuntimeProjectionController.allAgentsBusyState(),
  })
  const runtimeDebugLogger = createCliRuntimeDebugLogger({
    logger: deps.appLogger,
    debugLogsEnabled: deps.debugLogsEnabled,
    getResponseLayout: deps.multiAgentResponseLayout,
    splitAgentResponseMode,
    isAttached,
    getAgentCount: () => deps.sessionState().agents.length,
    getFocusedAgentId: focusedAgentId,
    hasTranscriptScrollbox: deps.transcriptScrollboxRefController.hasScrollbox,
    getVisibleTranscriptAgentId: visibleTranscriptAgentId,
  })
  const logProviderRunDebug = runtimeDebugLogger.logProviderRun
  const logViewDebug = runtimeDebugLogger.logView
  const logVisibleTranscriptOutput = runtimeDebugLogger.logVisibleTranscriptOutput
  const logFocusedBadgeChange = runtimeDebugLogger.logFocusedBadgeChange
  createEffect(() => {
    logViewDebug("state changed")
  })

  return {
    isAttached,
    focusedAgentId,
    multiAgentMode,
    workflowScreenShowing,
    splitAgentResponseMode,
    activeInteractionForAgent,
    focusedAgentInteraction,
    workflowPromptState,
    responsePaneSelection,
    responsePaneAgentSignature,
    responsePrimaryAgent,
    responseVisibleAgents,
    visibleTranscriptAgentId,
    responsePaneRows,
    primaryTranscriptSurfaceTone,
    auxiliaryTranscriptSurfaceTone,
    agentActivityLabel: agentRuntimeProjectionController.agentActivityLabel,
    focusedAgent: agentRuntimeProjectionController.focusedAgent,
    focusedBackendProvider: agentRuntimeProjectionController.focusedBackendProvider,
    focusedProviderRun: agentRuntimeProjectionController.focusedProviderRun,
    resolveSessionAgent: agentRuntimeProjectionController.resolveSessionAgent,
    agentBusyLatch: agentRuntimeProjectionController.agentBusyLatch,
    anyPromptWork: agentRuntimeProjectionController.anyPromptWork,
    hasPromptWorkByAgent: agentRuntimeProjectionController.hasPromptWorkByAgent,
    focusedQueueDepth: agentRuntimeProjectionController.focusedQueueDepth,
    focusedActivePrompt: agentRuntimeProjectionController.focusedActivePrompt,
    focusedActivityLabel: agentRuntimeProjectionController.focusedActivityLabel,
    markAgentBusy: agentRuntimeProjectionController.markAgentBusy,
    clearAgentBusy: agentRuntimeProjectionController.clearAgentBusy,
    focusedAgentBusy: agentRuntimeProjectionController.focusedAgentBusy,
    allAgentsBusyState: agentRuntimeProjectionController.allAgentsBusyState,
    setAgentActivityLabel: agentRuntimeProjectionController.setAgentActivityLabel,
    transcriptEntryProjectionController,
    visibleTranscriptEntries,
    connectedClientCount,
    activePrompt,
    focusedStatusBadge,
    runtimeDebugLogger,
    logProviderRunDebug,
    logViewDebug,
    logVisibleTranscriptOutput,
    logFocusedBadgeChange,
  }
}

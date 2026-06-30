import { BoxRenderable, TextAttributes } from "@opentui/core"
import { createEffect } from "solid-js"

import { createAgentInteractionStripController } from "./agent-interaction-strip-controller.js"
import { CHROME_UPDATE_THROTTLE_MS } from "./cli-runtime-tuning.js"
import { createCliLoadingStateController } from "./cli-loading-state-controller.js"
import { createPromptMetaRenderController } from "./prompt-meta-render-controller.js"
import { renderPromptMeta } from "./prompt-meta-renderer.js"
import { createResponseLayoutController } from "./response-layout-controller.js"
import { createResponsePaneRepaintController } from "./response-pane-repaint-controller.js"
import {
  createSessionChromeRenderController,
} from "./session-chrome-render-controller.js"
import {
  createSessionChromeSummaryRenderState,
  renderSessionChromeSummary,
} from "./session-chrome-summary-renderer.js"
import {
  createSessionChromeUpdateController,
} from "./session-chrome-update-controller.js"
import { agentHasPromptWork, sessionHasProjectedRuntimeState } from "./session-state.js"
import { createSplitPaneFooterRenderController } from "./split-pane-footer-render-controller.js"
import {
  renderSplitPaneFooters as renderSplitPaneFootersView,
} from "./split-pane-footer-renderer.js"
import { createStatusIndicatorController } from "./status-indicator-controller.js"
import {
  renderStatusIndicator as renderStatusIndicatorView,
} from "./status-indicator-renderer.js"
import {
  resolveTranscriptSurfaceTone,
  transcriptSurfacePalette,
} from "./transcript-render.js"
import { theme } from "./theme.js"
import { buildEmptyTranscriptRenderable } from "./workspace-renderables.js"
import { HOTKEY_TOGGLE_LABEL } from "./hotkeys.js"
import { STATUS_BADGE_WIDTH } from "./runtime.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"

type AnyFn = (...args: any[]) => any

export type CliResponseShellCompositionDeps = {
  renderer: any
  scheduleTimer: AnyFn
  clearTimer: AnyFn
  uiBatchController: {
    isBatched: AnyFn
  }
  splitPaneFooterRenderState: any
  statusIndicatorRenderState: any
  responsePaneRenderRefStore: any
  transcriptScrollboxRefController: {
    current: AnyFn
  }
  historyLoadingRenderController: {
    getBox: AnyFn
  }
  scheduleResponsePaneRepaint: AnyFn
  renderHistoryLoadingIndicator: AnyFn
  transcriptEntryProjectionController: {
    visibleEntryCount: AnyFn
  }
  transcriptRenderDeferralController: {
    request: AnyFn
  }
  isAttached: AnyFn
  workflowScreenActive: AnyFn
  maxAgentsPerScreen: AnyFn
  responseVisibleAgents: AnyFn
  focusedAgentId: AnyFn
  providerRunState: AnyFn
  currentProviderSelection: AnyFn
  agentActivityLabels: AnyFn
  hasPromptWorkByAgent: AnyFn
  streamingAgentId: AnyFn
  agentBusyLatch: AnyFn
  agentBusyLatches: AnyFn
  sessionState: AnyFn
  agentLocationLabel: AnyFn
  workingAnimationFrame: AnyFn
  activeInteractionForAgent: AnyFn
  queuedPromptStripItemsForAgent: AnyFn
  selectedQueuedPromptIndexForAgent: AnyFn
  onQueuedPromptAction: AnyFn
  interactionChoiceStore: any
  promptUsageMeta: AnyFn
  sessionHydrating: AnyFn
  setSessionHydrating: AnyFn
  setLoadingHistory: AnyFn
  setHistoryLoadingMessage: AnyFn
  rebuildTranscript: AnyFn
  focusedStatusBadge: AnyFn
  runtimeDebugLogger: {
    resetFocusedBadgeChange: AnyFn
  }
  logFocusedBadgeChange: AnyFn
  splitAgentResponseMode: AnyFn
  responsePaneRows: AnyFn
  responsePaneSelection: AnyFn
  workspaceScreenMode: AnyFn
  multiAgentResponseLayout: AnyFn
  terminalWidth: AnyFn
  responsePaneAgentSignature: AnyFn
  clearAuxiliaryAgentPane: AnyFn
  unregisterAgentScrollbox: AnyFn
  getCurrentAuxiliaryAgentId: AnyFn
  setCurrentAuxiliaryAgentId: AnyFn
  registerAgentScrollbox: AnyFn
  rebuildAuxiliaryAgentPane: AnyFn
  primaryTranscriptRuntimeStore: {
    getMountedTranscriptAgentId: AnyFn
  }
  agentPaneEntries: AnyFn
  replaceTranscriptEntries: AnyFn
  logViewDebug: AnyFn
  promptSubmissionAgentStateController: {
    clearSubmittingAgentId: AnyFn
  }
  setAgentBusyLatches: AnyFn
  providerRunStateSignal: AnyFn
  working: AnyFn
  activeStatusLabel: AnyFn
  providerActivityLabel: AnyFn
  syncPromptPlaceholder: AnyFn
  fatalError: AnyFn
  submitting: AnyFn
  footerHint: AnyFn
  connectedClientCount: AnyFn
  multiAgentMode: AnyFn
  sessionStatusMode: AnyFn
  workspaceLiveSyncStatus: AnyFn
  footerFlash: AnyFn
  promptMetaParts: AnyFn
}

export function createCliResponseShellComposition(deps: CliResponseShellCompositionDeps) {
  const splitPaneFooterRenderController = createSplitPaneFooterRenderController({
    renderer: deps.renderer,
    state: deps.splitPaneFooterRenderState,
    primaryBox: deps.responsePaneRenderRefStore.getPrimaryFooterBox,
    auxiliaryBoxes: deps.responsePaneRenderRefStore.getAuxiliaryFooterBoxes,
    isAttached: deps.isAttached,
    workflowScreenActive: () => deps.workflowScreenActive(),
    maxAgentsPerScreen: deps.maxAgentsPerScreen,
    visibleAgents: deps.responseVisibleAgents,
    metaagentTasks: () => deps.sessionState().metaagent_tasks ?? [],
    focusedAgentId: deps.focusedAgentId,
    providerRun: deps.providerRunState,
    currentProviderSelection: deps.currentProviderSelection,
    agentActivityLabels: deps.agentActivityLabels,
    hasPromptWorkByAgent: deps.hasPromptWorkByAgent,
    streamingAgentId: deps.streamingAgentId,
    agentBusyLatch: deps.agentBusyLatch,
    hasProjectedRuntimeState: () => sessionHasProjectedRuntimeState(deps.sessionState()),
    sessionConfigValues: () => deps.sessionState().config_state?.values,
    agentLocationLabel: deps.agentLocationLabel,
    badgeWidth: STATUS_BADGE_WIDTH,
    animationFrame: deps.workingAnimationFrame,
    renderFooters: renderSplitPaneFootersView,
  })
  const renderSplitPaneFooters = splitPaneFooterRenderController.render

  const agentInteractionStripController = createAgentInteractionStripController({
    renderer: deps.renderer,
    primaryBox: deps.responsePaneRenderRefStore.getPrimaryInteractionBox,
    auxiliaryBoxes: deps.responsePaneRenderRefStore.getAuxiliaryInteractionBoxes,
    visibleAgents: deps.responseVisibleAgents,
    maxAgentsPerScreen: deps.maxAgentsPerScreen,
    focusedAgentId: deps.focusedAgentId,
    activeInteractionForAgent: deps.activeInteractionForAgent,
    selectedChoiceIndex: deps.interactionChoiceStore.selectedChoiceIndex,
    setSelectedChoiceIndex: deps.interactionChoiceStore.setSelectedIndex,
    customReply: deps.interactionChoiceStore.customReply,
    customEditing: deps.interactionChoiceStore.isCustomEditing,
    queuedPromptStripItemsForAgent: deps.queuedPromptStripItemsForAgent,
    selectedQueuedPromptIndexForAgent: deps.selectedQueuedPromptIndexForAgent,
    onQueuedPromptAction: deps.onQueuedPromptAction,
    renderStrips: renderAgentInteractionStrips,
  })
  const renderAgentInteractions = agentInteractionStripController.render

  const promptMetaRenderController = createPromptMetaRenderController({
    getUsage: deps.promptUsageMeta,
    onRefAssigned: () => {
      updateSessionChrome()
    },
    renderMeta: renderPromptMeta,
  })

  const requestTranscriptRender = () => {
    deps.transcriptRenderDeferralController.request()
  }

  const loadingStateController = createCliLoadingStateController({
    getSessionHydrating: deps.sessionHydrating,
    setSessionHydrating: deps.setSessionHydrating,
    setLoadingHistory: deps.setLoadingHistory,
    setHistoryLoadingMessage: deps.setHistoryLoadingMessage,
    renderHistoryLoadingIndicator: deps.renderHistoryLoadingIndicator,
    isAttached: deps.isAttached,
    visibleTranscriptEntryCount: deps.transcriptEntryProjectionController.visibleEntryCount,
    workflowScreenActive: () => deps.workflowScreenActive(),
    rebuildTranscript: () => deps.rebuildTranscript(),
    requestTranscriptRender,
  })

  const statusIndicatorController = createStatusIndicatorController<BoxRenderable>({
    isAttached: deps.isAttached,
    getBadge: deps.focusedStatusBadge,
    getAnimationFrame: deps.workingAnimationFrame,
    resetFocusedBadgeChange: deps.runtimeDebugLogger.resetFocusedBadgeChange,
    logFocusedBadgeChange: deps.logFocusedBadgeChange,
    renderIndicator: ({ box, attached, badge, animationFrame }) => {
      renderStatusIndicatorView({
        renderer: deps.renderer,
        box,
        state: deps.statusIndicatorRenderState,
        attached,
        badge,
        badgeWidth: STATUS_BADGE_WIDTH,
        animationFrame,
      })
    },
  })
  const renderStatusIndicator = statusIndicatorController.render

  const responseLayoutController = createResponseLayoutController({
    getRefs: () => deps.responsePaneRenderRefStore.snapshot({
      primaryScrollbox: deps.transcriptScrollboxRefController.current(),
      historyLoadingBox: deps.historyLoadingRenderController.getBox(),
    }),
    getSplit: deps.splitAgentResponseMode,
    getVisibleAgents: deps.responseVisibleAgents,
    getPaneRows: deps.responsePaneRows,
    getFocusedAgentId: deps.focusedAgentId,
    getShowWorkflowScreen: () => deps.workflowScreenActive(),
    getMaxAgentsPerScreen: deps.maxAgentsPerScreen,
    getResponsePaneSelection: deps.responsePaneSelection,
    getTheme: () => theme,
    emptyTextAttributes: TextAttributes.NONE,
    panelBackgroundForFocus: (focused) => transcriptSurfacePalette(resolveTranscriptSurfaceTone(true, focused)).panel,
    renderSplitPaneFooters,
    renderAgentInteractions,
    clearAuxiliaryAgentPane: (agentId) => {
      deps.clearAuxiliaryAgentPane(agentId)
    },
    unregisterAgentScrollbox: deps.unregisterAgentScrollbox,
    getCurrentAuxiliaryAgentId: deps.getCurrentAuxiliaryAgentId,
    setCurrentAuxiliaryAgentId: deps.setCurrentAuxiliaryAgentId,
    registerAgentScrollbox: deps.registerAgentScrollbox,
    rebuildAuxiliaryAgentPane: (agentId) => {
      deps.rebuildAuxiliaryAgentPane(agentId)
    },
    buildEmptyTranscriptRenderable: () => buildEmptyTranscriptRenderable(deps.renderer),
    getMountedTranscriptAgentId: deps.primaryTranscriptRuntimeStore.getMountedTranscriptAgentId,
    getAgentPaneEntries: (agentId) => deps.agentPaneEntries()[agentId] ?? [],
    replaceTranscriptEntries: (nextEntries, agentId) => {
      deps.replaceTranscriptEntries(nextEntries, agentId)
    },
    scheduleResponsePaneRepaint: deps.scheduleResponsePaneRepaint,
    logViewDebug: deps.logViewDebug,
  })
  const applyResponseLayout = responseLayoutController.apply

  createEffect(() => {
    deps.splitAgentResponseMode()
    deps.workspaceScreenMode()
    deps.multiAgentResponseLayout()
    deps.maxAgentsPerScreen()
    deps.terminalWidth()
    deps.responsePaneAgentSignature()
    deps.focusedAgentId()
    applyResponseLayout()
  })

  createEffect(() => {
    if (deps.isAttached()) {
      return
    }
    deps.promptSubmissionAgentStateController.clearSubmittingAgentId()
    deps.setAgentBusyLatches({})
  })

  createEffect(() => {
    deps.providerRunStateSignal()?.model
    deps.providerRunStateSignal()?.variant
    deps.working()
    deps.activeStatusLabel()
    deps.providerActivityLabel()
    deps.streamingAgentId()
    deps.workspaceLiveSyncStatus()
    deps.agentBusyLatches()
    for (const agent of deps.sessionState().agents) {
      agent.is_processing
      agent.state
    }
    deps.agentActivityLabels()
    updateSessionChrome()
  })

  const responsePaneRepaintController = createResponsePaneRepaintController({
    scheduleTimer: deps.scheduleTimer,
    repaint: () => {
      applyResponseLayout()
      deps.scheduleResponsePaneRepaint()
    },
  })

  const sessionChromeRenderController = createSessionChromeRenderController({
    renderer: deps.renderer,
    createSummaryRenderState: createSessionChromeSummaryRenderState,
    renderSummary: (options) => {
      renderSessionChromeSummary(options as unknown as Parameters<typeof renderSessionChromeSummary>[0])
    },
    syncPromptPlaceholder: deps.syncPromptPlaceholder,
    getFatalError: deps.fatalError,
    getSubmitting: deps.submitting,
    getFooterHint: deps.footerHint,
    isAttached: deps.isAttached,
    getSession: deps.sessionState,
    getConnectedClientCount: deps.connectedClientCount,
    getMultiAgentMode: deps.multiAgentMode,
    getResponseLayout: deps.multiAgentResponseLayout,
    getSessionStatusMode: deps.sessionStatusMode,
    getFocusedHasPromptWork: () => agentHasPromptWork(deps.sessionState(), deps.focusedAgentId()),
    getWorkspaceLiveSyncStatus: deps.workspaceLiveSyncStatus,
    getHotkeyToggleLabel: () => HOTKEY_TOGGLE_LABEL,
    getFooterFlash: deps.footerFlash,
    getPromptMetaParts: deps.promptMetaParts,
    setPromptMetaRenderables: promptMetaRenderController.setRenderables,
    renderStatusIndicator,
    renderSplitPaneFooters,
    renderAgentInteractions,
    getWorking: deps.working,
    getActiveStatusLabel: deps.activeStatusLabel,
    getProviderActivityLabel: deps.providerActivityLabel,
    getStreamingAgentId: deps.streamingAgentId,
  })

  const sessionChromeUpdateController = createSessionChromeUpdateController({
    delayMs: CHROME_UPDATE_THROTTLE_MS,
    scheduleTimer: deps.scheduleTimer,
    clearTimer: deps.clearTimer,
    isBatched: deps.uiBatchController.isBatched,
    applyUpdate: sessionChromeRenderController.apply,
  })
  const updateSessionChrome = () => {
    sessionChromeUpdateController.request(sessionChromeRenderController.shouldThrottle())
  }

  return {
    renderSplitPaneFooters,
    renderAgentInteractions,
    assignPromptMetaRef: promptMetaRenderController.assignRefCallback,
    requestTranscriptRender,
    setHistoryLoadingState: loadingStateController.setHistoryLoadingState,
    setSessionHydratingState: loadingStateController.setSessionHydratingState,
    applyResponseLayout,
    refreshSplitPaneFocusRepaint: responsePaneRepaintController.refreshFocus,
    renderSessionChromeBoundary: sessionChromeUpdateController.flush,
    updateSessionChrome,
    sessionChromeUpdateController,
    assignStatusIndicatorBox: statusIndicatorController.assignBox,
    assignFooterSummaryBox: sessionChromeRenderController.assignFooterSummaryBox,
  }
}

import { onMount } from "solid-js"

import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import { clampScrollTop } from "./history-viewport.js"
import { createPrimaryTranscriptEntryController } from "./primary-transcript-entry-controller.js"
import { createPrimaryTranscriptRenderController } from "./primary-transcript-render-controller.js"
import { getSessionHistory } from "./session-history-api.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"
import {
  buildTranscriptEntryRenderable,
  transcriptRenderMode,
} from "./transcript-render.js"
import {
  buildEmptyTranscriptRenderable,
  buildLoadingTranscriptRenderable,
  buildNoSessionRenderable,
  buildWorkflowOutlineRenderable,
} from "./workspace-renderables.js"

type AnyFn = (...args: any[]) => any

export type CliPrimaryTranscriptCompositionDeps = {
  client: any
  bootstrap: any
  renderer: any
  appLogger: any
  formatError: AnyFn
  scheduleTimer: AnyFn
  isAttached: AnyFn
  sessionHydrating: AnyFn
  loadingHistory: AnyFn
  nextHistoryCursor: AnyFn
  setNextHistoryCursor: AnyFn
  entryCounter: AnyFn
  setEntryCounter: AnyFn
  setHistoryLoadingState: AnyFn
  setEntries: AnyFn
  setPromptHistoryEntries: AnyFn
  setPromptHistoryIndex: AnyFn
  setPromptHistoryDraft: AnyFn
  setProviderCatalogState: AnyFn
  setProviderCommandCatalogState: AnyFn
  updateSessionChrome: AnyFn
  flashFooter: AnyFn
  attachmentState: AnyFn
  sessionState: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  setSelectedWorkflowNodeId: AnyFn
  workflowScreenActive: AnyFn
  workflowInspector: AnyFn
  workspaceShellEntries: AnyFn
  workspaceShellContext: AnyFn
  waitingRoomState: AnyFn
  availableSessions: AnyFn
  providerCatalogState: AnyFn
  waitingRoomCloudNotice: AnyFn
  waitingRoomInventoryStatus: AnyFn
  relayStatusState: AnyFn
  remoteMachinesState: AnyFn
  remoteKernelsState: AnyFn
  terminalsState: AnyFn
  waitingRoomTargets: AnyFn
  themeRegistryState: AnyFn
  transcriptScrollboxRefController: {
    current: AnyFn
  }
  primaryTranscriptRuntimeStore: {
    getEmptyRenderable: AnyFn
    setEmptyRenderable: AnyFn
    transcriptRenderables: any
    clearTools: AnyFn
    setMountedTranscriptAgentId: AnyFn
    setLastScrollTop: AnyFn
    getLastScrollTop: AnyFn
  }
  transcriptEntryProjectionController: {
    renderableEntries: AnyFn
  }
  visibleTranscriptAgentId: AnyFn
  transcriptSyntaxStyleController: {
    current: AnyFn
  }
  historyScrollRestoreController: {
    restorePrependedHistory: AnyFn
    isRestoring: AnyFn
  }
  transcriptTurnStateController: {
    setCurrentTurnId: AnyFn
    setNextTurnId: AnyFn
  }
  expandedTurnIdsForAgent: AnyFn
  syncVisibleTranscriptPreview: AnyFn
  toggleTurn: AnyFn
  toggleBlob: AnyFn
  primaryTranscriptSurfaceTone: AnyFn
  requestTranscriptRender: AnyFn
  requestRootRender: AnyFn
  logViewDebug: AnyFn
  promptHistoryHydrationController: any
  splitAgentResponseMode: AnyFn
  maxAgentsPerScreen: AnyFn
  setAgentPaneEntries: AnyFn
  setAgentPanePreview: AnyFn
}

export function createCliPrimaryTranscriptComposition(deps: CliPrimaryTranscriptCompositionDeps) {
  const primaryTranscriptRenderController = createPrimaryTranscriptRenderController({
    getScrollbox: deps.transcriptScrollboxRefController.current,
    getEmptyRenderable: deps.primaryTranscriptRuntimeStore.getEmptyRenderable,
    setEmptyRenderable: deps.primaryTranscriptRuntimeStore.setEmptyRenderable,
    renderables: deps.primaryTranscriptRuntimeStore.transcriptRenderables,
    visibleEntries: deps.transcriptEntryProjectionController.renderableEntries,
    workflowScreenActive: () => deps.workflowScreenActive(),
    showWorkflowOutline: () => deps.isAttached() && deps.workflowScreenActive(),
    buildWorkflowRenderable: () => buildWorkflowOutlineRenderable(deps.renderer, {
      workflows: deps.sessionState().workflows ?? [],
      agents: deps.sessionState().agents,
      workflowRuns: deps.sessionState().workflow_runs ?? [],
      selectedWorkflowId: deps.selectedWorkflowId(),
      selectedNodeId: deps.selectedWorkflowNodeId(),
      onSelectNode: (nodeId) => {
        deps.setSelectedWorkflowNodeId(nodeId)
        rebuildTranscript()
      },
      inspector: deps.workflowInspector(),
      shellPane: {
        entries: deps.workspaceShellEntries(),
        sessionId: deps.workspaceShellContext().sessionId ?? null,
        agentId: deps.workspaceShellContext().agentId ?? null,
      },
    }),
    buildEmptyRenderable: () => deps.isAttached()
      ? (deps.sessionHydrating()
          ? buildLoadingTranscriptRenderable(deps.renderer)
          : buildEmptyTranscriptRenderable(deps.renderer))
      : buildNoSessionRenderable(deps.renderer, deps.waitingRoomState(), deps.availableSessions(), deps.providerCatalogState(), {
        cloudNotice: deps.waitingRoomCloudNotice(),
        inventoryStatus: deps.waitingRoomInventoryStatus(),
        loadingFrame: deps.waitingRoomState().introStep,
        relay: deps.relayStatusState(),
        machines: deps.remoteMachinesState(),
        kernels: deps.remoteKernelsState(),
        terminals: deps.terminalsState(),
      }, deps.waitingRoomTargets(), deps.themeRegistryState()),
    buildEntryRenderable: (entry) => buildTranscriptEntryRenderable(
      deps.renderer,
      entry,
      deps.transcriptSyntaxStyleController.current(),
      deps.toggleTurn,
      deps.toggleBlob,
      deps.primaryTranscriptSurfaceTone(),
    ),
    renderMode: transcriptRenderMode,
    requestTranscriptRender: deps.requestTranscriptRender,
    requestRendererRender: deps.requestRootRender,
    shouldResetEmptyScrollTop: deps.isAttached,
    clampScrollTop,
    setLastScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
    logViewDebug: deps.logViewDebug,
  })
  const mountTranscriptEntry = primaryTranscriptRenderController.mountEntry
  const reconcileMountedTranscript = primaryTranscriptRenderController.reconcileMountedTranscript
  const updateTranscriptEntry = primaryTranscriptRenderController.updateEntry
  const rebuildTranscript = primaryTranscriptRenderController.rebuildTranscript

  const primaryTranscriptEntryController = createPrimaryTranscriptEntryController({
    getScrollbox: deps.transcriptScrollboxRefController.current,
    getEntries: deps.transcriptEntryProjectionController.renderableEntries,
    getVisibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    expandedTurnIdsForAgent: deps.expandedTurnIdsForAgent,
    clearToolState: deps.primaryTranscriptRuntimeStore.clearTools,
    setEntries: deps.setEntries,
    setEntryCounter: deps.setEntryCounter,
    setCurrentTurnId: deps.transcriptTurnStateController.setCurrentTurnId,
    setNextTurnId: deps.transcriptTurnStateController.setNextTurnId,
    setMountedTranscriptAgentId: deps.primaryTranscriptRuntimeStore.setMountedTranscriptAgentId,
    setLastScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
    rebuildTranscript,
    syncVisibleTranscriptPreview: deps.syncVisibleTranscriptPreview,
    restorePrependedHistory: (request) => deps.historyScrollRestoreController.restorePrependedHistory(request),
  })
  const replaceTranscriptEntries = primaryTranscriptEntryController.replaceEntries
  const prependTranscriptEntries = primaryTranscriptEntryController.prependEntries

  const attachedSessionPrimeController = createAttachedSessionPrimeController({
    promptHistoryHydrationController: deps.promptHistoryHydrationController,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    maxAgentsPerScreen: deps.maxAgentsPerScreen,
    loadVisibleAgentHistory: (sessionId, agentId) => getSessionHistory(deps.client, sessionId, null, agentId),
    setAgentPaneEntries: (agentId, nextEntries) => {
      deps.setAgentPaneEntries((current: any) => ({
        ...current,
        [agentId]: nextEntries,
      }))
    },
    setAgentPanePreview: deps.setAgentPanePreview,
    replaceTranscriptEntries,
    setNextHistoryCursor: deps.setNextHistoryCursor,
  })
  const primeAttachedSessionBinding = attachedSessionPrimeController.prime

  const deferredBootstrapController = createDeferredBootstrapController({
    getDeferred: () => deps.bootstrap.deferred,
    currentAttachmentSessionId: () => deps.attachmentState()?.session_id ?? null,
    currentTranscriptEntryCount: () => deps.transcriptEntryProjectionController.renderableEntries().length,
    entryCounter: deps.entryCounter,
    setProviderCatalog: deps.setProviderCatalogState,
    setProviderCommandCatalogs: deps.setProviderCommandCatalogState,
    updateSessionChrome: deps.updateSessionChrome,
    setPromptHistoryEntries: deps.setPromptHistoryEntries,
    resetPromptHistoryNavigation: () => {
      deps.setPromptHistoryIndex(null)
      deps.setPromptHistoryDraft(null)
    },
    setNextHistoryCursor: deps.setNextHistoryCursor,
    setAgentPaneEntries: (agentId, nextEntries) => {
      deps.setAgentPaneEntries((current: any) => ({
        ...current,
        [agentId]: nextEntries,
      }))
    },
    setAgentPanePreview: deps.setAgentPanePreview,
    replaceTranscriptEntries,
    prependTranscriptEntries,
    logWarning: (message, fields) => {
      deps.appLogger?.warn(message, fields)
    },
    formatError: deps.formatError,
  })

  onMount(() => {
    deferredBootstrapController.apply()
  })

  const transcriptHistoryLoadController = createTranscriptHistoryLoadController({
    isAttached: deps.isAttached,
    isLoading: deps.loadingHistory,
    getCursor: deps.nextHistoryCursor,
    getSessionId: () => deps.sessionState().id,
    getVisibleAgentId: deps.visibleTranscriptAgentId,
    getEntryCounter: deps.entryCounter,
    setLoading: deps.setHistoryLoadingState,
    setNextCursor: deps.setNextHistoryCursor,
    loadPage: (sessionId, cursor, agentId) => getSessionHistory(deps.client, sessionId, cursor, agentId),
    prependEntries: prependTranscriptEntries,
    flashError: (message) => {
      deps.flashFooter(message, "error")
    },
    logWarning: (message, fields) => {
      deps.appLogger?.warn(message, fields)
    },
    formatError: deps.formatError,
  })
  const transcriptHistoryAutoloadController = createTranscriptHistoryAutoloadController({
    scheduleTimer: deps.scheduleTimer,
    getScrollbox: deps.transcriptScrollboxRefController.current,
    isScrollRestoring: () => deps.historyScrollRestoreController.isRestoring(),
    isAttached: deps.isAttached,
    isLoadingHistory: deps.loadingHistory,
    hasMoreHistory: () => deps.nextHistoryCursor() !== null,
    getLastScrollTop: deps.primaryTranscriptRuntimeStore.getLastScrollTop,
    setLastScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
    loadOlderHistory: () => transcriptHistoryLoadController.loadOlderPage(),
  })

  return {
    mountTranscriptEntry,
    reconcileMountedTranscript,
    updateTranscriptEntry,
    rebuildTranscript,
    replaceTranscriptEntries,
    prependTranscriptEntries,
    primeAttachedSessionBinding,
    bumpHistoryLoadGeneration: transcriptHistoryLoadController.bumpGeneration,
    transcriptHistoryAutoloadController,
  }
}

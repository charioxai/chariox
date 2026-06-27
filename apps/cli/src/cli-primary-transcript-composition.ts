import { onMount } from "solid-js"

import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import { createDeferredBootstrapController } from "./deferred-bootstrap-controller.js"
import { clampScrollTop } from "./history-viewport.js"
import { createPrimaryTranscriptEntryController } from "./primary-transcript-entry-controller.js"
import { createPrimaryTranscriptRenderController } from "./primary-transcript-render-controller.js"
import { hydrateOutlineAgentEntries } from "./session-history-outline.js"
import { getSessionHistoryOutline } from "./session-history-api.js"
import { createTranscriptHistoryAutoloadController } from "./transcript-history-autoload-controller.js"
import { reindexTranscriptEntries } from "./transcript-text.js"
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
  setTerminalCommandCatalogState: AnyFn
  updateSessionChrome: AnyFn
  flashFooter: AnyFn
  attachmentState: AnyFn
  sessionState: AnyFn
  selectedWorkflowId: AnyFn
  selectedWorkflowNodeId: AnyFn
  setSelectedWorkflowNodeId: AnyFn
  selectedWorkflowComponent: AnyFn
  setSelectedWorkflowComponent: AnyFn
  setWorkflowInspectorMode: AnyFn
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
  externalProviderSessionsState: AnyFn
  externalProviderSessionsPageState: AnyFn
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
  onQueuedPromptAction: AnyFn
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
      selectedComponent: deps.selectedWorkflowComponent(),
      onSelectNode: (nodeId) => {
        deps.setSelectedWorkflowNodeId(nodeId)
        deps.setSelectedWorkflowComponent(nodeId ? { kind: "node", id: nodeId } : { kind: "workflow" })
        deps.setWorkflowInspectorMode(nodeId ? "trace" : "logs")
        rebuildTranscript()
      },
      onSelectComponent: (selection, backingNodeId) => {
        deps.setSelectedWorkflowComponent(selection)
        deps.setSelectedWorkflowNodeId(backingNodeId)
        deps.setWorkflowInspectorMode(selection.kind === "workflow" ? "logs" : "trace")
        rebuildTranscript()
      },
      inspector: deps.workflowInspector(),
      transcriptRendering: {
        syntaxStyle: deps.transcriptSyntaxStyleController.current(),
        onToggleTurn: deps.toggleTurn,
        onToggleBlob: deps.toggleBlob,
        surfaceTone: deps.primaryTranscriptSurfaceTone(),
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
        externalProviderSessions: deps.externalProviderSessionsState(),
        externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
        externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
      }, deps.waitingRoomTargets(), deps.themeRegistryState()),
    buildEntryRenderable: (entry) => buildTranscriptEntryRenderable(
      deps.renderer,
      entry,
      deps.transcriptSyntaxStyleController.current(),
      deps.toggleTurn,
      deps.toggleBlob,
      deps.primaryTranscriptSurfaceTone(),
      deps.onQueuedPromptAction,
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

  const hasMoreVisibleHistory = () => {
    const cursorState = deps.nextHistoryCursor()
    const visibleAgentId = deps.visibleTranscriptAgentId()
    return Boolean(
      cursorState
        && visibleAgentId
        && cursorState.agentId === visibleAgentId,
    )
  }

  const loadOlderVisibleHistory = async () => {
    const cursorState = deps.nextHistoryCursor()
    const visibleAgentId = deps.visibleTranscriptAgentId()
    const session = deps.sessionState()
    if (!cursorState || !visibleAgentId || cursorState.agentId !== visibleAgentId || !session?.id) {
      return false
    }

    deps.setHistoryLoadingState(true)
    try {
      const outline = await getSessionHistoryOutline(
        deps.client,
        session.id,
        [visibleAgentId],
        4,
        cursorState.cursor,
      )
      const outlineAgent = outline.agents.find((agent) => agent.agent_id === visibleAgentId)
      deps.setNextHistoryCursor(
        outlineAgent?.next_cursor
          ? { agentId: visibleAgentId, cursor: outlineAgent.next_cursor }
          : null,
      )
      const olderEntries = outlineAgent ? hydrateOutlineAgentEntries(outlineAgent) : []
      if (olderEntries.length === 0) {
        return false
      }
      await prependTranscriptEntries(reindexTranscriptEntries(olderEntries, deps.entryCounter()))
      return true
    } catch (error) {
      deps.appLogger?.warn("failed to load older transcript history", {
        error: deps.formatError(error),
        sessionId: session.id,
        agentId: visibleAgentId,
      })
      return false
    } finally {
      deps.setHistoryLoadingState(false)
    }
  }

  const attachedSessionPrimeController = createAttachedSessionPrimeController({
    promptHistoryHydrationController: deps.promptHistoryHydrationController,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    maxAgentsPerScreen: deps.maxAgentsPerScreen,
    loadSessionHistoryOutline: (sessionId, agentIds) => getSessionHistoryOutline(deps.client, sessionId, agentIds, 4),
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
    setTerminalCommandCatalog: deps.setTerminalCommandCatalogState,
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
  queueMicrotask(() => deferredBootstrapController.apply())

  const transcriptHistoryAutoloadController = createTranscriptHistoryAutoloadController({
    scheduleTimer: deps.scheduleTimer,
    getScrollbox: deps.transcriptScrollboxRefController.current,
    isScrollRestoring: () => deps.historyScrollRestoreController.isRestoring(),
    isAttached: deps.isAttached,
    isLoadingHistory: deps.loadingHistory,
    getLastScrollTop: deps.primaryTranscriptRuntimeStore.getLastScrollTop,
    setLastScrollTop: deps.primaryTranscriptRuntimeStore.setLastScrollTop,
    hasMoreHistory: hasMoreVisibleHistory,
    loadOlderHistory: loadOlderVisibleHistory,
  })

  return {
    mountTranscriptEntry,
    reconcileMountedTranscript,
    updateTranscriptEntry,
    rebuildTranscript,
    replaceTranscriptEntries,
    prependTranscriptEntries,
    primeAttachedSessionBinding,
    bumpHistoryLoadGeneration: () => undefined,
    transcriptHistoryAutoloadController,
  }
}

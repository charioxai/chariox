import { createEffect } from "solid-js"

import { createAgentPaneRefreshController } from "./agent-pane-refresh-controller.js"
import { createAgentPaneStoreController } from "./agent-pane-store-controller.js"
import { createAgentPaneTranscriptEntryController } from "./agent-pane-transcript-entry-controller.js"
import { createAgentPaneTranscriptInteractionController } from "./agent-pane-transcript-interaction-controller.js"
import { createAgentPaneTranscriptRenderController } from "./agent-pane-transcript-render-controller.js"
import { createAgentPaneTranscriptRetentionController } from "./agent-pane-transcript-retention-controller.js"
import { createAgentPaneTranscriptStreamController } from "./agent-pane-transcript-stream-controller.js"
import { createAgentPaneStreamingCommitController } from "./agent-pane-streaming-commit-controller.js"
import {
  LIVE_TRANSCRIPT_LIMIT,
  LIVE_TRANSCRIPT_MAX_CHARS,
} from "./cli-runtime-tuning.js"
import { clampScrollTop } from "./history-viewport.js"
import { formatTranscriptPreview } from "@arroba/kernel-client/session-history-preview"
import { buildEmptyTranscriptRenderable } from "./workspace-renderables.js"
import {
  getSessionHistoryBlobContent,
  getSessionHistoryOutline,
} from "./session-history-api.js"
import { hydrateSessionHistoryOutlineAgentEntries } from "@arroba/kernel-client/session-history-transcript"
import { splitPaneAuxiliaryAgentIds } from "@arroba/kernel-client/response-pane-selection"
import {
  buildTranscriptEntryRenderable,
  transcriptRenderMode,
} from "./transcript-render.js"

type AnyFn = (...args: any[]) => any

export type CliAgentPaneCompositionDeps = {
  client: any
  renderer: any
  isAttached: AnyFn
  visibleTranscriptAgentId: AnyFn
  visibleTranscriptEntries: AnyFn
  agentPaneEntries: AnyFn
  setAgentPaneEntries: AnyFn
  setAgentPanePreviews: AnyFn
  setExpandedTurnIdsByAgent: AnyFn
  setNextHistoryCursor: AnyFn
  sessionState: AnyFn
  focusedAgentId: AnyFn
  maxAgentsPerScreen: AnyFn
  splitAgentResponseMode: AnyFn
  responsePrimaryAgent: AnyFn
  expandedTurnIdsByAgent: AnyFn
  expandedTurnIdsForAgent: AnyFn
  setExpandedTurnState: AnyFn
  applyExpandedTurns: AnyFn
  retainPromptFocus: AnyFn
  formatError: AnyFn
  agentPaneRuntimeStore: {
    scrollboxes: any
    entryRenderables: any
    emptyRenderables: any
    toolStates: any
  }
  transcriptSyntaxStyleController: {
    current: AnyFn
  }
  auxiliaryTranscriptSurfaceTone: AnyFn
  onQueuedPromptAction: AnyFn
  renderScheduler: {
    requestRenderable: AnyFn
  }
  primaryTranscriptRuntimeStore: {
    getMountedTranscriptAgentId: AnyFn
  }
  replaceTranscriptEntries: AnyFn
  applyResponseLayout: AnyFn
}

export function createCliAgentPaneComposition(deps: CliAgentPaneCompositionDeps) {
  const agentPaneStoreController = createAgentPaneStoreController({
    isAttached: deps.isAttached,
    getVisibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    getVisibleTranscriptEntries: deps.visibleTranscriptEntries,
    getPaneEntriesByAgent: deps.agentPaneEntries,
    updatePaneEntries: (updater) => {
      deps.setAgentPaneEntries((current: any) => updater(current))
    },
    updatePanePreviews: (updater) => {
      deps.setAgentPanePreviews((current: any) => updater(current))
    },
    getSessionAgents: () => deps.sessionState().agents,
    getFocusedAgentId: deps.focusedAgentId,
    getMaxAgentsPerScreen: deps.maxAgentsPerScreen,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    getPrimaryAgentId: () => deps.responsePrimaryAgent()?.id ?? null,
    expandedTurnIdsForAgent: deps.expandedTurnIdsForAgent,
    replaceTranscriptEntries: (nextEntries, agentId) => {
      deps.replaceTranscriptEntries(nextEntries, agentId)
    },
    reconcileMountedAuxiliaryTranscript: (agentId, previousPaneEntries, sanitizedEntries) => {
      reconcileMountedAuxiliaryTranscript(agentId, previousPaneEntries, sanitizedEntries)
    },
  })
  const setAgentPanePreview = agentPaneStoreController.setAgentPanePreview
  const persistVisibleTranscriptEntries = agentPaneStoreController.persistVisibleTranscriptEntries
  const setAgentTranscriptEntries = agentPaneStoreController.setAgentTranscriptEntries
  const visibleAuxiliaryAgentIds = agentPaneStoreController.visibleAuxiliaryAgentIds
  const commitAgentPaneEntries = agentPaneStoreController.commitAgentPaneEntries
  const currentAgentPaneEntries = agentPaneStoreController.currentAgentPaneEntries

  const agentPaneTranscriptEntryController = createAgentPaneTranscriptEntryController({
    currentAgentPaneEntries,
    visibleTranscriptAgentId: deps.visibleTranscriptAgentId,
    visibleTranscriptEntries: deps.visibleTranscriptEntries,
    expandedTurnIdsForAgent: deps.expandedTurnIdsForAgent,
    setAgentPanePreview,
    updateAgentPanePreviews: (updater) => {
      deps.setAgentPanePreviews((current: any) => updater(current))
    },
    trimLiveAgentPaneEntries: (agentId, nextEntries) => trimLiveAgentPaneEntries(agentId, nextEntries),
    setAgentTranscriptEntries: (agentId, nextEntries, turnIds) => {
      setAgentTranscriptEntries(agentId, nextEntries, turnIds ? [...turnIds] : undefined)
    },
  })
  const hasTrailingUserPrompt = agentPaneTranscriptEntryController.hasTrailingUserPrompt

  const agentPaneTranscriptInteractionController = createAgentPaneTranscriptInteractionController({
    currentAgentPaneEntries,
    expandedTurnIdsForAgent: deps.expandedTurnIdsForAgent,
    setExpandedTurnState: deps.setExpandedTurnState,
    commitAgentPaneEntries: (agentId, nextEntries) => {
      commitAgentPaneEntries(agentId, nextEntries)
    },
    reconcileMountedAuxiliaryTranscript: (agentId, currentEntries, nextEntries) => {
      reconcileMountedAuxiliaryTranscript(agentId, currentEntries, nextEntries)
    },
    retainPromptFocus: deps.retainPromptFocus,
    loadHistoryBlobContent: (agentId, blobId) => getSessionHistoryBlobContent(
      deps.client,
      deps.sessionState().id,
      agentId,
      blobId,
    ),
    formatError: deps.formatError,
  })
  const toggleAuxiliaryPaneTurn = agentPaneTranscriptInteractionController.toggleTurn
  const toggleAuxiliaryPaneBlob = agentPaneTranscriptInteractionController.toggleBlob

  const agentPaneTranscriptRenderController = createAgentPaneTranscriptRenderController({
    scrollboxes: deps.agentPaneRuntimeStore.scrollboxes,
    entryRenderables: deps.agentPaneRuntimeStore.entryRenderables,
    emptyRenderables: deps.agentPaneRuntimeStore.emptyRenderables,
    toolStates: deps.agentPaneRuntimeStore.toolStates,
    paneEntries: (agentId) => deps.agentPaneEntries()[agentId] ?? [],
    buildEmptyRenderable: () => buildEmptyTranscriptRenderable(deps.renderer),
    buildEntryRenderable: (agentId, entry) => buildTranscriptEntryRenderable(
      deps.renderer,
      entry,
      deps.transcriptSyntaxStyleController.current(),
      (turnId, nextToggleEntryId) => toggleAuxiliaryPaneTurn(agentId, turnId, nextToggleEntryId),
      (entryId, collapsed) => toggleAuxiliaryPaneBlob(agentId, entryId, collapsed),
      deps.auxiliaryTranscriptSurfaceTone(agentId),
      deps.onQueuedPromptAction,
    ),
    renderMode: transcriptRenderMode,
    requestRenderable: (renderable) => deps.renderScheduler.requestRenderable(renderable),
    clampScrollTop,
    activeAgentIdsForSession: (session: any) => splitPaneAuxiliaryAgentIds(
      session.agents,
      session.focused_agent_id,
      true,
      deps.maxAgentsPerScreen(),
    ),
  })
  const auxiliaryAgentPaneTools = agentPaneTranscriptRenderController.toolStateForAgent
  const clearAuxiliaryAgentPane = agentPaneTranscriptRenderController.clearPane
  const rebuildAuxiliaryAgentPane = agentPaneTranscriptRenderController.rebuildPane
  const updateAuxiliaryTranscriptEntry = agentPaneTranscriptRenderController.updateEntry
  const reconcileMountedAuxiliaryTranscript = agentPaneTranscriptRenderController.reconcileMountedTranscript
  const pruneAuxiliaryAgentPanes = agentPaneTranscriptRenderController.prunePanes

  const agentPaneTranscriptRetentionController = createAgentPaneTranscriptRetentionController({
    maxEntries: LIVE_TRANSCRIPT_LIMIT,
    maxChars: LIVE_TRANSCRIPT_MAX_CHARS,
    deleteToolForMergeKey: (agentId, mergeKey) => {
      auxiliaryAgentPaneTools(agentId).delete(mergeKey)
    },
  })
  const trimLiveAgentPaneEntries = agentPaneTranscriptRetentionController.trimLiveEntries

  const agentPaneStreamingCommitController = createAgentPaneStreamingCommitController({
    trimLiveAgentPaneEntries,
    expandedTurnIdsForAgent: deps.expandedTurnIdsForAgent,
    commitAgentPaneEntries,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    getResponsePrimaryAgentId: () => deps.responsePrimaryAgent()?.id ?? null,
    replaceTranscriptEntries: (nextEntries, agentId) => {
      deps.replaceTranscriptEntries(nextEntries, agentId)
    },
    visibleAuxiliaryAgentIds,
    updateAuxiliaryTranscriptEntry,
    reconcileMountedAuxiliaryTranscript,
  })
  const commitStreamingAgentPaneEntry = agentPaneStreamingCommitController.commitStreamingEntry

  const syncVisibleTranscriptPreview = agentPaneTranscriptEntryController.syncVisibleTranscriptPreview
  const appendAgentPanePreview = agentPaneTranscriptEntryController.appendPreview
  const appendTranscriptEntryToAgentPane = agentPaneTranscriptEntryController.appendEntry

  const agentPaneTranscriptStreamController = createAgentPaneTranscriptStreamController({
    currentAgentPaneEntries,
    trimLiveAgentPaneEntries,
    setAgentTranscriptEntries,
    commitStreamingAgentPaneEntry,
    toolStateForAgent: (agentId) => auxiliaryAgentPaneTools(agentId) as any,
  })
  const appendProviderChunkToAgentPane = agentPaneTranscriptStreamController.appendProviderChunk
  const appendToolUpdateToAgentPane = agentPaneTranscriptStreamController.appendToolUpdate

  createEffect(() => {
    if (!deps.isAttached()) {
      return
    }
    const agentId = deps.responsePrimaryAgent()?.id ?? null
    const currentEntries = deps.visibleTranscriptEntries().map((entry: any) => ({ ...entry }))
    if (!agentId || agentId !== deps.primaryTranscriptRuntimeStore.getMountedTranscriptAgentId()) {
      return
    }
    deps.setAgentPaneEntries((current: any) => ({
      ...current,
      [agentId]: currentEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(currentEntries))
  })

  const agentPaneRefreshController = createAgentPaneRefreshController({
    getCurrentAgents: () => deps.sessionState().agents,
    getFocusedAgentId: deps.focusedAgentId,
    getExpandedTurnIdsByAgent: deps.expandedTurnIdsByAgent,
    currentAgentPaneEntries,
    splitAgentResponseMode: deps.splitAgentResponseMode,
    maxAgentsPerScreen: deps.maxAgentsPerScreen,
    loadHistoryPage: async (sessionId, agentId, cursor) => {
      const outline = await getSessionHistoryOutline(deps.client, sessionId, [agentId], 4, cursor)
      const outlineAgent = outline.agents.find((agent) => agent.agent_id === agentId)
      return {
        entries: outlineAgent ? hydrateSessionHistoryOutlineAgentEntries(outlineAgent) : [],
        nextCursor: outlineAgent?.next_cursor ?? null,
      }
    },
    pruneAuxiliaryAgentPanes,
    setExpandedTurnIdsByAgent: deps.setExpandedTurnIdsByAgent,
    setAgentPanePreviews: deps.setAgentPanePreviews,
    setAgentPaneEntries: deps.setAgentPaneEntries,
    setNextHistoryCursor: deps.setNextHistoryCursor,
    applyExpandedTurns: deps.applyExpandedTurns,
    replaceTranscriptEntries: (entries, agentId) => deps.replaceTranscriptEntries(entries, agentId),
    applyResponseLayout: deps.applyResponseLayout,
    rebuildAuxiliaryAgentPane,
  })
  const refreshAgentPanes = agentPaneRefreshController.refresh
  const shouldRefreshAgentPanesForSessionChange = agentPaneRefreshController.shouldRefreshForSessionChange

  return {
    auxiliaryAgentPaneTools,
    clearAllAuxiliaryAgentPanes: agentPaneTranscriptRenderController.clearAll,
    clearAuxiliaryAgentPane,
    rebuildAuxiliaryAgentPane,
    updateAuxiliaryTranscriptEntry,
    reconcileMountedAuxiliaryTranscript,
    pruneAuxiliaryAgentPanes,
    trimLiveAgentPaneEntries,
    setAgentPanePreview,
    persistVisibleTranscriptEntries,
    setAgentTranscriptEntries,
    visibleAuxiliaryAgentIds,
    commitAgentPaneEntries,
    currentAgentPaneEntries,
    hasTrailingUserPrompt,
    toggleAuxiliaryPaneTurn,
    toggleAuxiliaryPaneBlob,
    syncVisibleTranscriptPreview,
    appendAgentPanePreview,
    appendTranscriptEntryToAgentPane,
    appendProviderChunkToAgentPane,
    appendToolUpdateToAgentPane,
    refreshAgentPanes,
    shouldRefreshAgentPanesForSessionChange,
  }
}

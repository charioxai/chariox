import {
  selectCurrentAgentPaneEntries,
} from "@arroba/kernel-client/agent-pane-state"
import {
  splitPaneAuxiliaryAgentIds,
} from "@arroba/kernel-client/response-pane-selection"
import type {
  AgentInstance,
  TranscriptEntry,
} from "./cli-types.js"
import { applyTranscriptDisplayState } from "./transcript-display.js"
import { formatTranscriptPreview } from "@arroba/kernel-client/session-history-preview"

export type AgentPaneStoreControllerDeps = {
  isAttached: () => boolean
  getVisibleTranscriptAgentId: () => string | null
  getVisibleTranscriptEntries: () => TranscriptEntry[]
  getPaneEntriesByAgent: () => Record<string, TranscriptEntry[]>
  updatePaneEntries: (
    updater: (current: Record<string, TranscriptEntry[]>) => Record<string, TranscriptEntry[]>,
  ) => void
  updatePanePreviews: (
    updater: (current: Record<string, string>) => Record<string, string>,
  ) => void
  getSessionAgents: () => readonly AgentInstance[]
  getFocusedAgentId: () => string | null
  getMaxAgentsPerScreen: () => number
  splitAgentResponseMode: () => boolean
  getPrimaryAgentId: () => string | null
  expandedTurnIdsForAgent: (agentId: string) => number[]
  replaceTranscriptEntries: (entries: TranscriptEntry[], agentId: string) => void
  reconcileMountedAuxiliaryTranscript: (
    agentId: string,
    previousEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
  ) => void
}

export function createAgentPaneStoreController(deps: AgentPaneStoreControllerDeps) {
  const setAgentPanePreview = (agentId: string, text: string) => {
    deps.updatePanePreviews((current) => ({
      ...current,
      [agentId]: text,
    }))
  }

  const commitAgentPaneEntries = (agentId: string, nextEntries: TranscriptEntry[]) => {
    const persistedEntries = cloneEntries(nextEntries)
    deps.updatePaneEntries((current) => ({
      ...current,
      [agentId]: persistedEntries,
    }))
    setAgentPanePreview(agentId, formatTranscriptPreview(persistedEntries))
  }

  const visibleAuxiliaryAgentIds = () => splitPaneAuxiliaryAgentIds(
    deps.getSessionAgents(),
    deps.getFocusedAgentId(),
    true,
    deps.getMaxAgentsPerScreen(),
  )

  return {
    setAgentPanePreview,
    persistVisibleTranscriptEntries(nextEntries: TranscriptEntry[]) {
      const agentId = deps.getVisibleTranscriptAgentId()
      if (!deps.isAttached() || !agentId) {
        return
      }
      commitAgentPaneEntries(agentId, nextEntries)
    },
    setAgentTranscriptEntries(
      agentId: string,
      nextEntries: TranscriptEntry[],
      turnIds = deps.expandedTurnIdsForAgent(agentId),
    ) {
      const previousPaneEntries = deps.getPaneEntriesByAgent()[agentId] ?? []
      const sanitizedEntries = applyTranscriptDisplayState(nextEntries.filter(Boolean), turnIds)
      commitAgentPaneEntries(agentId, sanitizedEntries)
      if (deps.splitAgentResponseMode() && agentId === deps.getPrimaryAgentId()) {
        deps.replaceTranscriptEntries(cloneEntries(sanitizedEntries), agentId)
      }
      if (deps.splitAgentResponseMode() && visibleAuxiliaryAgentIds().includes(agentId)) {
        deps.reconcileMountedAuxiliaryTranscript(agentId, previousPaneEntries, sanitizedEntries)
      }
    },
    visibleAuxiliaryAgentIds,
    commitAgentPaneEntries,
    currentAgentPaneEntries(agentId: string) {
      return selectCurrentAgentPaneEntries({
        agentId,
        visibleAgentId: deps.getVisibleTranscriptAgentId(),
        visibleEntries: deps.getVisibleTranscriptEntries(),
        paneEntriesByAgent: deps.getPaneEntriesByAgent(),
      })
    },
  }
}

function cloneEntries(entries: TranscriptEntry[]) {
  return entries.map((entry) => ({ ...entry }))
}

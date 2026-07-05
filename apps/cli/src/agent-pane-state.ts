import {
  refreshAgentPaneState as sharedRefreshAgentPaneState,
  selectCurrentAgentPaneEntries as sharedSelectCurrentAgentPaneEntries,
  shouldRefreshAgentPanesForSessionChange as sharedShouldRefreshAgentPanesForSessionChange,
  trimAgentPaneEntries as sharedTrimAgentPaneEntries,
  type AgentPaneExternalProviderImport,
  type AgentPaneRefreshResult as SharedAgentPaneRefreshResult,
  type AgentPaneSession,
} from "@arroba/kernel-client/agent-pane-state"

export type { AgentPaneExternalProviderImport, AgentPaneSession }

export type AgentPaneRefreshResult<TEntry, TCursor> = SharedAgentPaneRefreshResult<TEntry, TCursor>

export function selectCurrentAgentPaneEntries<TEntry extends object>(options: {
  agentId: string
  visibleAgentId: string | null
  visibleEntries: readonly TEntry[]
  paneEntriesByAgent: Record<string, TEntry[]>
}) {
  return sharedSelectCurrentAgentPaneEntries(options)
}

export function shouldRefreshAgentPanesForSessionChange<TAgent extends { id: string }>(options: {
  previousAgents: readonly TAgent[]
  nextAgents: readonly TAgent[]
  splitAgentResponseMode: boolean
  currentFocusedAgentId: string | null
  nextFocusedAgentId: string | null
}): boolean {
  return sharedShouldRefreshAgentPanesForSessionChange(options)
}

export function trimAgentPaneEntries<TEntry extends { text: string; mergeKey?: string }>(options: {
  entries: TEntry[]
  maxEntries: number
  maxChars: number
  onTrimmedMergeKey?: (mergeKey: string) => void
}) {
  return sharedTrimAgentPaneEntries(options)
}

export const refreshAgentPaneState = sharedRefreshAgentPaneState

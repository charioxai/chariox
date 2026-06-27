import type { RuntimeSession } from "./cli-types.js"
import { getToolActivityLabel } from "./runtime.js"
import { agentHasPromptWork } from "./session-state.js"

export type ToolActivityUpdate = {
  tool?: string | null
  status?: string | null
}

export type AgentBusyState = {
  id: string
  busy: boolean
}

export function readAgentBusyLatch(
  latches: Record<string, boolean>,
  agentId: string | null | undefined,
): boolean {
  return agentId ? (latches[agentId] ?? false) : false
}

export function nextAgentBusyLatches(
  current: Record<string, boolean>,
  agentId: string | null | undefined,
  busy: boolean,
): Record<string, boolean> {
  if (!agentId || (current[agentId] ?? false) === busy) {
    return current
  }
  if (busy) {
    return {
      ...current,
      [agentId]: true,
    }
  }
  const next = { ...current }
  delete next[agentId]
  return next
}

export function shouldPreserveAgentActivityLabel(options: {
  agentId: string | null | undefined
  session: RuntimeSession
  streamingAgentId: string | null
}): boolean {
  const agentId = options.agentId
  if (!agentId) {
    return false
  }
  return options.streamingAgentId === agentId
    || agentHasPromptWork(options.session, agentId)
    || (!options.session.agent_activity
      && !options.session.prompt_states
      && options.session.agents.some((agent) => agent.id === agentId && (agent.is_processing || agent.state === "Working")))
}

export function nextAgentActivityLabels(
  current: Record<string, string | null>,
  agentId: string | null | undefined,
  nextLabel: string | null,
  preserveCurrent: boolean,
): Record<string, string | null> {
  if (!agentId) {
    return current
  }
  return {
    ...current,
    [agentId]: nextLabel ?? (preserveCurrent ? (current[agentId] ?? null) : null),
  }
}

export function resolveActiveToolLabelForAgent(options: {
  agentId: string | null | undefined
  visibleTranscriptAgentId: string | null
  activeToolLabels: Iterable<string>
  agentPaneToolUpdates: Iterable<ToolActivityUpdate> | null | undefined
}): string | null {
  const agentId = options.agentId
  if (!agentId) {
    return null
  }
  if (agentId === options.visibleTranscriptAgentId) {
    return Array.from(options.activeToolLabels).at(-1) ?? null
  }
  const labels = Array.from(options.agentPaneToolUpdates ?? [])
    .filter((update) => update.status !== "completed" && update.status !== "error" && update.status !== "cancelled")
    .map((update) => getToolActivityLabel(update.tool))
    .filter((label): label is string => Boolean(label))
  return labels.at(-1) ?? null
}

export function deriveFocusedActivityLabel(options: {
  focusedAgentId: string | null
  activeToolLabel: string | null
  agentActivityLabel: string | null
}): string | null {
  return options.focusedAgentId ? (options.activeToolLabel ?? options.agentActivityLabel) : null
}

export function deriveFocusedAgentBusy(options: {
  focusedAgentId: string | null
  submitting: boolean
  submittingAgentId: string | null
  session: RuntimeSession
  streamingAgentId: string | null
  focusedActivityLabel: string | null
  agentBusyLatches: Record<string, boolean>
}): boolean {
  const agentId = options.focusedAgentId
  if (!agentId) {
    return false
  }
  const focused = options.session.agents.find((agent) => agent.id === agentId) ?? null
  const allowLegacyProcessing = !options.session.agent_activity && !options.session.prompt_states
  return (options.submitting && options.submittingAgentId === agentId)
    || agentHasPromptWork(options.session, agentId)
    || options.streamingAgentId === agentId
    || Boolean(options.focusedActivityLabel)
    || readAgentBusyLatch(options.agentBusyLatches, agentId)
    || Boolean(allowLegacyProcessing && focused && (focused.is_processing || focused.state === "Working"))
}

export function deriveAllAgentsBusyState(options: {
  submitting: boolean
  submittingAgentId: string | null
  session: RuntimeSession
  streamingAgentId: string | null
  agentActivityLabels: Record<string, string | null>
  agentBusyLatches: Record<string, boolean>
}): AgentBusyState[] {
  return options.session.agents.map((agent) => {
    const agentId = agent.id
    const allowLegacyProcessing = !options.session.agent_activity && !options.session.prompt_states
    const isBusy = (options.submitting && options.submittingAgentId === agentId)
      || agentHasPromptWork(options.session, agentId)
      || options.streamingAgentId === agentId
      || Boolean(options.agentActivityLabels[agentId])
      || readAgentBusyLatch(options.agentBusyLatches, agentId)
      || (allowLegacyProcessing && (agent.is_processing || agent.state === "Working"))
    return { id: agentId, busy: isBusy }
  })
}

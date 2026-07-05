import {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  shouldPreserveAgentActivityLabel as shouldPreserveAgentActivityLabelState,
  type AgentToolActivityUpdate as ToolActivityUpdate,
  type AgentBusyState,
} from "@arroba/kernel-client/session-runtime-transition"
import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  agentPromptState,
  focusedProviderRunForAgent,
  promptWorkByAgent,
  sessionHasPromptWork,
} from "./session-state.js"
import {
  resolveSessionAgentReference,
  type ResolvedAgentReference,
} from "@arroba/kernel-client/session-agent-resolver"
import {
  isBackendProviderId,
  normalizeBackendProviderId,
  type BackendProviderId,
} from "./provider-catalog.js"

export type AgentRuntimeProjectionControllerDeps = {
  getSession: () => RuntimeSession
  getFocusedAgentId: () => string | null
  getProviderRun: () => RuntimeProviderRun | null
  getVisibleTranscriptAgentId: () => string | null
  getActiveToolLabels: () => Iterable<string>
  getAgentPaneToolUpdates: (agentId: string) => Iterable<ToolActivityUpdate> | null | undefined
  getAgentPanePreviews: () => Record<string, string>
  getAgentActivityLabels: () => Record<string, string | null>
  updateAgentActivityLabels: (
    updater: (current: Record<string, string | null>) => Record<string, string | null>,
  ) => void
  getAgentBusyLatches: () => Record<string, boolean>
  updateAgentBusyLatches: (
    updater: (current: Record<string, boolean>) => Record<string, boolean>,
  ) => void
  getSubmitting: () => boolean
  getSubmittingAgentId: () => string | null
  getStreamingAgentId: () => string | null
}

export function createAgentRuntimeProjectionController(
  deps: AgentRuntimeProjectionControllerDeps,
) {
  const agentPanePreview = (agentId: string) => deps.getAgentPanePreviews()[agentId] ?? ""
  const agentActivityLabel = (agentId: string | null | undefined) =>
    agentId ? deps.getAgentActivityLabels()[agentId] ?? null : null
  const focusedAgent = (): AgentInstance | null =>
    deps.getSession().agents.find((agent) => agent.id === deps.getFocusedAgentId()) ?? null
  const focusedBackendProvider = (): BackendProviderId | null => {
    const provider = focusedAgent()?.provider
    return provider && (isBackendProviderId(provider) || provider === "claude")
      ? normalizeBackendProviderId(provider)
      : null
  }
  const focusedProviderRun = () => focusedProviderRunForAgent(
    deps.getProviderRun(),
    deps.getFocusedAgentId(),
  )
  const resolveSessionAgent = (reference?: string | null): ResolvedAgentReference =>
    resolveSessionAgentReference(deps.getSession(), deps.getFocusedAgentId(), reference)
  const promptStateForAgent = (agentId: string | null | undefined) =>
    agentPromptState(deps.getSession(), agentId)
  const agentQueuedDepth = (agentId: string | null | undefined) =>
    promptStateForAgent(agentId)?.queued_prompts.length ?? 0
  const agentActivePrompt = (agentId: string | null | undefined) =>
    promptStateForAgent(agentId)?.active_prompt ?? null
  const agentBusyLatch = (agentId: string | null | undefined) =>
    readAgentBusyLatch(deps.getAgentBusyLatches(), agentId)
  const anyPromptWork = () => sessionHasPromptWork(deps.getSession())
  const hasPromptWorkByAgent = () => promptWorkByAgent(deps.getSession())
  const focusedPromptState = () => promptStateForAgent(deps.getFocusedAgentId())
  const focusedQueueDepth = () => agentQueuedDepth(deps.getFocusedAgentId())
  const focusedActivePrompt = () => agentActivePrompt(deps.getFocusedAgentId())
  const activeToolLabelForAgent = (agentId: string | null | undefined) => {
    return resolveActiveToolLabelForAgent({
      agentId,
      visibleTranscriptAgentId: deps.getVisibleTranscriptAgentId(),
      activeToolLabels: deps.getActiveToolLabels(),
      agentPaneToolUpdates: agentId ? deps.getAgentPaneToolUpdates(agentId) : null,
    })
  }
  const focusedActivityLabel = () => {
    const agentId = deps.getFocusedAgentId()
    return deriveFocusedActivityLabel({
      focusedAgentId: agentId,
      activeToolLabel: activeToolLabelForAgent(agentId),
      agentActivityLabel: agentActivityLabel(agentId),
    })
  }
  const setAgentBusyLatch = (agentId: string | null | undefined, busy: boolean) => {
    deps.updateAgentBusyLatches((current) => nextAgentBusyLatches(current, agentId, busy))
  }
  const focusedAgentBusy = () => deriveFocusedAgentBusy({
    focusedAgentId: deps.getFocusedAgentId(),
    submitting: deps.getSubmitting(),
    submittingAgentId: deps.getSubmittingAgentId(),
    session: deps.getSession(),
    streamingAgentId: deps.getStreamingAgentId(),
    focusedActivityLabel: focusedActivityLabel(),
    agentBusyLatches: deps.getAgentBusyLatches(),
  })
  const allAgentsBusyState = (): AgentBusyState[] => deriveAllAgentsBusyState({
    submitting: deps.getSubmitting(),
    submittingAgentId: deps.getSubmittingAgentId(),
    session: deps.getSession(),
    streamingAgentId: deps.getStreamingAgentId(),
    agentActivityLabels: deps.getAgentActivityLabels(),
    agentBusyLatches: deps.getAgentBusyLatches(),
  })
  const shouldPreserveAgentActivityLabel = (agentId: string | null | undefined) => {
    return shouldPreserveAgentActivityLabelState({
      agentId,
      session: deps.getSession(),
      streamingAgentId: deps.getStreamingAgentId(),
    })
  }
  const setAgentActivityLabel = (agentId: string | null | undefined, nextLabel: string | null) => {
    deps.updateAgentActivityLabels((current) => nextAgentActivityLabels(
      current,
      agentId,
      nextLabel,
      shouldPreserveAgentActivityLabel(agentId),
    ))
  }

  return {
    agentPanePreview,
    agentActivityLabel,
    focusedAgent,
    focusedBackendProvider,
    focusedProviderRun,
    resolveSessionAgent,
    promptStateForAgent,
    agentQueuedDepth,
    agentActivePrompt,
    agentBusyLatch,
    anyPromptWork,
    hasPromptWorkByAgent,
    focusedPromptState,
    focusedQueueDepth,
    focusedActivePrompt,
    activeToolLabelForAgent,
    focusedActivityLabel,
    setAgentBusyLatch,
    markAgentBusy: (agentId: string | null | undefined) => setAgentBusyLatch(agentId, true),
    clearAgentBusy: (agentId: string | null | undefined) => setAgentBusyLatch(agentId, false),
    focusedAgentBusy,
    allAgentsBusyState,
    shouldPreserveAgentActivityLabel,
    setAgentActivityLabel,
  }
}

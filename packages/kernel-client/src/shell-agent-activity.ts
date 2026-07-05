import type {
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
  WorkspaceLiveSyncStatus,
} from "./kernel-types.js"
import { workspaceLiveSyncFooterSummary } from "./shell-workspace-format.js"
export {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
  type ActivePromptLifecycleRecord,
  type PromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
export {
  sessionActivePromptForAgent,
  sessionActivePromptIdForAgent,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptStateForAgent,
} from "./session-prompt-identity.js"
export {
  sessionAgentIsBusy,
  sessionHasAgentRuntimeProjection,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
  type SessionPromptWorkSummary,
} from "./session-prompt-work.js"
export {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  resolveSessionStreamingAgentId,
  sessionFocusedAgentId,
  sessionRuntimeTransitionState,
  sessionShouldConfirmIdleTurnCompletion,
  sessionWorkingStateAfterPromptWork,
  shouldPreserveAgentActivityLabel,
  turnCompletionDelayMs,
  type AgentBusyState,
  type AgentToolActivityUpdate,
  type SessionIdleTurnCompletionInput,
  type SessionRuntimeTransitionOptions,
  type SessionRuntimeTransitionState,
  type SessionStreamingAgent,
  type TurnCompletionDelayInput,
} from "./session-runtime-transition.js"
export {
  agentRuntimeStateFromProjection,
  sessionAgentHasUnreadIdleOutput,
  sessionAgentPaneStatusBadge,
  sessionAgentRuntimeActivityProjection,
  sessionAgentRuntimeActivityStatus,
  sessionAgentRuntimeDisplayState,
  sessionAgentRuntimeState,
  sessionFocusedStatusBadge,
  sessionStatusLabel,
  sessionStatusMode,
  type AgentPromptStateLike,
  type AgentRuntimeDisplayState,
  type AgentRuntimeProjectionContext,
  type SessionAgentBusyState,
  type SessionAgentPaneStatusBadge,
  type SessionAgentPaneStatusInput,
  type SessionFocusedStatusBadge,
  type SessionStatusBadgePart,
  type SessionStatusBadgeTone,
  type SessionStatusMode,
} from "./session-runtime-status.js"

import type { SessionStatusMode } from "./session-runtime-status.js"

export function sessionFooterHint(options: {
  readonly fatalError: string | null
  readonly activePromptId: string | null
  readonly queueDepth: number
  readonly statusLine: string
}): string {
  if (options.fatalError) {
    return options.fatalError
  }
  if (options.activePromptId) {
    return options.queueDepth > 0
      ? `Processing ${options.activePromptId}; ${options.queueDepth} queued.`
      : `Processing ${options.activePromptId}.`
  }
  if (options.queueDepth > 0) {
    return `${options.queueDepth} queued prompt${options.queueDepth === 1 ? "" : "s"}.`
  }
  return options.statusLine
}

export function sessionAttachedFooterSummary(options: {
  readonly session: RuntimeSession
  readonly connectedClientCount: number
  readonly multiAgentMode: boolean
  readonly sessionStatusMode: SessionStatusMode
  readonly hotkeyToggleLabel: string
  readonly workspaceLiveSyncStatus?: WorkspaceLiveSyncStatus | null
}): string {
  const navigationInfo = options.multiAgentMode ? " • Tab cycles focus • Ctrl+P opens workflow" : ""
  const agentInfo = sessionVisibleAgentSummary(options.session)
  const workspaceLiveSyncInfo = options.workspaceLiveSyncStatus
    ? ` • ${workspaceLiveSyncFooterSummary(options.workspaceLiveSyncStatus)}`
    : ""

  return `Session ${options.session.alias ?? options.session.id} • ${options.connectedClientCount} ${options.connectedClientCount === 1 ? "CLI" : "CLIs"} connected • ${agentInfo}${workspaceLiveSyncInfo}${options.sessionStatusMode === "working" ? " • Ctrl+C to stop" : ""}${navigationInfo} • ${options.hotkeyToggleLabel} hotkeys`
}

export function sessionVisibleAgentSummary(session: RuntimeSession): string {
  const counts = session.collaboration_agent_counts
  const visibleCount = counts?.owned_agent_count ?? session.agents.length
  const otherCount = counts?.other_user_agent_count ?? 0
  const collaboratorCount = counts?.collaborator_count ?? 0
  const ownLabel = `${visibleCount} visible ${visibleCount === 1 ? "agent" : "agents"}`
  const parts = [ownLabel]

  if (otherCount > 0) {
    parts.push(`${otherCount} collaborator ${otherCount === 1 ? "agent" : "agents"}`)
  }
  if (collaboratorCount > 0) {
    parts.push(`${collaboratorCount} ${collaboratorCount === 1 ? "collaborator" : "collaborators"}`)
  }

  return parts.join(" • ")
}

export function sessionActiveInteractionForAgent(
  session: Pick<RuntimeSession, "active_interactions">,
  agentId: string | null | undefined,
): RuntimeInteraction | null {
  if (!agentId) {
    return null
  }
  return session.active_interactions?.find((interaction) => interaction.agent_id === agentId) ?? null
}

export function runtimeProviderRunForAgent(
  run: RuntimeProviderRun | null,
  agentId: string | null | undefined,
): RuntimeProviderRun | null {
  return run && run.agent_instance_id === agentId ? run : null
}

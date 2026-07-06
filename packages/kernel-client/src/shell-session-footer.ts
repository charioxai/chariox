import type {
  RuntimeSession,
  WorkspaceLiveSyncStatus,
} from "./kernel-types.js"
import {
  sessionStatusMode,
  type SessionStatusMode,
} from "./session-runtime-status.js"
import { workspaceLiveSyncFooterSummary } from "./shell-workspace-format.js"

export type SessionChromeProjection = {
  readonly sessionStatusMode: SessionStatusMode
  readonly footerHint: string
}

export function sessionChromeProjection(options: {
  readonly daemonDisconnected: boolean
  readonly working: boolean
  readonly hasActiveTurnWork: boolean
  readonly submitting: boolean
  readonly queueDepth: number
  readonly fatalError: string | null
  readonly activePromptId: string | null
  readonly statusLine: string
}): SessionChromeProjection {
  return {
    sessionStatusMode: sessionStatusMode({
      daemonDisconnected: options.daemonDisconnected,
      working: options.working,
      hasActiveTurnWork: options.hasActiveTurnWork,
      submitting: options.submitting,
      queueDepth: options.queueDepth,
    }),
    footerHint: sessionFooterHint({
      fatalError: options.fatalError,
      activePromptId: options.activePromptId,
      queueDepth: options.queueDepth,
      statusLine: options.statusLine,
    }),
  }
}

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

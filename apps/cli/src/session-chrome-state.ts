import type { AgentInstance, RuntimeProviderRun, RuntimeSession, WorkspaceLiveSyncStatus } from "./cli-types.js"
import {
  applyProviderRunProfileToSession as sharedApplyProviderRunProfileToSession,
  derivePromptProviderSelection,
  providerRunForPromptSelection,
  resolveProviderModelContextLimit,
  type PromptProviderSelectionOptions,
  type PromptProviderSelection,
} from "@arroba/kernel-client/prompt-provider-selection"
import {
  sessionAttachedFooterSummary,
  sessionFooterHint,
} from "@arroba/kernel-client/shell-session-footer"
import {
  sessionFocusedStatusBadge,
  sessionStatusMode,
  type SessionAgentBusyState,
  type SessionFocusedStatusBadge,
  type SessionStatusMode as KernelSessionStatusMode,
  type SessionStatusBadgePart,
} from "@arroba/kernel-client/session-runtime-status"
import { deriveVisibleActivityLabel as sharedDeriveVisibleActivityLabel } from "@arroba/kernel-client/provider-status"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  formatPromptMetaParts,
  formatPromptUsageMeta,
  type PromptMetaPart,
  type PromptUsageMeta,
} from "./prompt-meta.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type SessionStatusMode = KernelSessionStatusMode
export type StatusBadgePart = SessionStatusBadgePart
export type FocusedStatusBadge = SessionFocusedStatusBadge

type ProviderSelectionOptions = {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  waitingRoomState: WaitingRoomState
  defaultProvider?: string
  defaultModel: string
  defaultEffort: string
}

export function deriveCurrentProviderSelection(options: ProviderSelectionOptions): PromptProviderSelection {
  return derivePromptProviderSelection(options as PromptProviderSelectionOptions)
}

export function applyProviderRunProfileToSession(
  session: RuntimeSession,
  providerRun: RuntimeProviderRun | null,
): RuntimeSession {
  return sharedApplyProviderRunProfileToSession(session, providerRun)
}

export function derivePromptMetaState(options: ProviderSelectionOptions): PromptMetaPart[] {
  const selection = deriveCurrentProviderSelection(options)
  return formatPromptMetaParts(
    selection.provider,
    selection.model,
    selection.effort,
  )
}

export function derivePromptUsageState(options: {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  catalog: ProviderCatalog
}): PromptUsageMeta | null {
  const run = providerRunForPromptSelection(options.providerRun, options.focusedAgent)
  if (!run) {
    return null
  }

  return formatPromptUsageMeta(
    run.usage_tokens_total,
    run.usage?.context_tokens,
    resolveProviderModelContextLimit(options.catalog, run.provider, run.model),
    12,
  )
}

export function deriveSessionStatusMode(options: {
  daemonDisconnected: boolean
  working: boolean
  hasActivePrompt: boolean
  submitting: boolean
  queueDepth: number
}): SessionStatusMode {
  return sessionStatusMode(options)
}

export function deriveFooterHint(options: {
  fatalError: string | null
  activePromptId: string | null
  queueDepth: number
  statusLine: string
}): string {
  return sessionFooterHint(options)
}

export function deriveVisibleActivityLabel(options: {
  providerActivityLabel: string | null
  activeToolLabels: Iterable<string>
}) {
  return sharedDeriveVisibleActivityLabel(options)
}

export type AgentBusyState = SessionAgentBusyState

export function deriveFocusedStatusBadge(options: {
  attached: boolean
  daemonDisconnected: boolean
  activeStatusLabel: string | null
  focusedBusy: boolean
  agents?: AgentBusyState[]
}): FocusedStatusBadge {
  return sessionFocusedStatusBadge(options)
}

export function deriveAttachedFooterSummary(options: {
  session: RuntimeSession
  connectedClientCount: number
  multiAgentMode: boolean
  responseLayout: MultiAgentResponseLayout
  sessionStatusMode: SessionStatusMode
  hotkeyToggleLabel: string
  focusedHasPromptWork?: boolean
  workspaceLiveSyncStatus?: WorkspaceLiveSyncStatus | null
}): string {
  return sessionAttachedFooterSummary(options)
}

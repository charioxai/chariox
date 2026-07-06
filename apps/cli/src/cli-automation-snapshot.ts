import type { ShellContext } from "@arroba/kernel-client/shell-core"
import {
  sessionHistoryEntryIsExternalProviderObserved,
} from "@arroba/kernel-client/external-provider-observation"
import {
  sessionAllowsLegacyAgentProcessingState,
} from "@arroba/kernel-client/session-prompt-work"
import {
  sessionAgentPaneStatusBadge,
  sessionAgentRuntimeDisplayStateByAgent,
} from "@arroba/kernel-client/session-runtime-status"

import type {
  RuntimeSession,
  SliceRecord,
  TranscriptEntry,
  ExternalProviderSessionRecord,
} from "./cli-types.js"
import type { CliAutomationSnapshot } from "./cli-automation.js"
import type { QueuedPromptStripItem } from "@arroba/kernel-client/queued-prompt-strip-state"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { ThemeRegistry } from "./theme-registry.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
} from "./waiting-room-inventory-api.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
import type { WaitingRoomState, WaitingRoomTargetState } from "./waiting-room-types.js"
import {
  renderWorkspaceShellTranscript,
  type WorkspaceShellEntry,
} from "./workspace-shell.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"

export type CliAutomationSnapshotDeps = {
  workspaceScreenMode: () => WorkspaceScreenMode
  workflowScreenActive: () => boolean
  daemonDisconnected: () => boolean
  statusLine: () => string
  sessionState: () => RuntimeSession
  focusedAgentId: () => string | null
  agentActivityLabels: () => Record<string, string | null>
  hasPromptWorkByAgent: () => Record<string, boolean>
  streamingAgentId: () => string | null
  agentBusyLatch: (agentId: string) => boolean
  isAttached: () => boolean
  waitingRoomState: () => WaitingRoomState
  availableSessions: () => SessionListEntry[]
  providerCatalogState: () => ProviderCatalog
  waitingRoomCloudNotice: () => string | null
  waitingRoomInventoryStatus: () => "loading" | "ready" | "error"
  relayStatusState: () => RelayStatusView | null
  remoteMachinesState: () => RemoteMachineView[]
  remoteKernelsState: () => RemoteKernelView[]
  terminalsState: () => TerminalView[]
  externalProviderSessionsState: () => ExternalProviderSessionRecord[]
  externalProviderSessionsPageState: () => { hasMore: boolean; nextCursor: string | null }
  slicesState: () => SliceRecord[]
  waitingRoomTargets: () => WaitingRoomTargetState
  themeRegistryState: () => ThemeRegistry
  selectedWorkflowId: () => string | null
  selectedWorkflowNodeId: () => string | null
  workspaceShellContext: () => ShellContext
  workspaceShellEntries: () => WorkspaceShellEntry[]
  transcriptEntries: () => TranscriptEntry[]
  visibleTranscriptAgentId: () => string | null
  agentPaneEntries: () => Record<string, TranscriptEntry[]>
  queuedPromptStripItemsForAgent?: (agentId: string | null | undefined) => readonly QueuedPromptStripItem[]
  selectedQueuedPromptIndexForAgent?: (agentId: string | null | undefined) => number
  footerFlash: () => unknown
  interactionChoiceSelection: (interactionId: string) => number
  interactionCustomReply: (interactionId: string) => string
  interactionCustomEditing: (interactionId: string) => boolean
}

export function buildCliAutomationSnapshot(deps: CliAutomationSnapshotDeps): CliAutomationSnapshot {
  const session = deps.sessionState()
  const selectedWorkflow = session.workflows?.find((workflow) => workflow.id === deps.selectedWorkflowId()) ?? null
  const waitingRoomState = deps.waitingRoomState()
  const shellEntries = deps.workspaceShellEntries()
  const agentRuntimeDisplayStates = sessionAgentRuntimeDisplayStateByAgent(session)
  return {
    screen: deps.workspaceScreenMode(),
    workflowScreenActive: deps.workflowScreenActive(),
    daemonDisconnected: deps.daemonDisconnected(),
    statusLine: deps.statusLine(),
    session: {
      id: session.id,
      workspace: session.workspace_id,
      worktree: session.worktree_id,
      focusedAgentId: deps.focusedAgentId(),
      agentCount: session.agents.length,
      agents: session.agents.map((agent) => {
        const badge = sessionAgentPaneStatusBadge({
          agent,
          activeLabel: deps.agentActivityLabels()[agent.id] ?? null,
          hasPromptWork: deps.hasPromptWorkByAgent()[agent.id] ?? false,
          isStreaming: agent.id === deps.streamingAgentId(),
          busyLatch: deps.agentBusyLatch(agent.id),
          useLegacyAgentProcessingState: sessionAllowsLegacyAgentProcessingState(session),
        })
        return {
          id: agent.id,
          alias: agent.alias,
          provider: agent.provider,
          state: agentRuntimeDisplayStates[agent.id],
          isProcessing: agentRuntimeDisplayStates[agent.id] === "Working",
          badge,
        }
      }),
    },
    interactions: (session.active_interactions ?? []).map((interaction) => ({
      id: interaction.id,
      agentId: interaction.agent_id,
      kind: interaction.kind,
      level: interaction.level,
      title: interaction.title,
      message: interaction.message,
      timeoutSec: interaction.timeout_sec,
      defaultOnTimeout: interaction.default_on_timeout,
      focused: deps.focusedAgentId() === interaction.agent_id,
      selectedChoiceIndex: deps.interactionChoiceSelection(interaction.id),
      customChoice: interaction.custom_choice ?? null,
      customReply: deps.interactionCustomReply(interaction.id),
      customEditing: deps.interactionCustomEditing(interaction.id),
      choices: interaction.choices.map((choice) => ({
        id: choice.id,
        label: choice.label,
        style: choice.style,
      })),
    })),
    waitingRoom: !deps.isAttached()
      ? {
        state: waitingRoomState,
        rows: waitingRoomRows(waitingRoomState, deps.availableSessions(), deps.providerCatalogState(), {
          cloudNotice: deps.waitingRoomCloudNotice(),
          inventoryStatus: deps.waitingRoomInventoryStatus(),
          loadingFrame: waitingRoomState.introStep,
          relay: deps.relayStatusState(),
          machines: deps.remoteMachinesState(),
          kernels: deps.remoteKernelsState(),
          terminals: deps.terminalsState(),
          externalProviderSessions: deps.externalProviderSessionsState(),
          externalProviderSessionsHasMore: deps.externalProviderSessionsPageState().hasMore,
          externalProviderSessionsNextCursor: deps.externalProviderSessionsPageState().nextCursor,
          slices: deps.slicesState(),
        }, deps.waitingRoomTargets(), deps.themeRegistryState()).map((row) => ({
          id: row.id,
          externalSessionId: row.id.startsWith("external-session:") ? row.id.slice("external-session:".length) : null,
          title: row.title,
          value: row.value,
          focused: row.focused,
          selectable: row.selectable,
        })),
      }
      : null,
    selectedWorkflowId: deps.selectedWorkflowId(),
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId(),
    selectedWorkflow: selectedWorkflow
      ? {
        id: selectedWorkflow.id,
        alias: selectedWorkflow.alias,
        nodeCount: selectedWorkflow.nodes?.length ?? 0,
        edgeCount: selectedWorkflow.edges?.length ?? 0,
        endpointCount: selectedWorkflow.endpoints?.length ?? 0,
      }
      : null,
    workflows: (session.workflows ?? []).map((workflow) => ({
      id: workflow.id,
      alias: workflow.alias,
      nodeCount: workflow.nodes?.length ?? 0,
      edgeCount: workflow.edges?.length ?? 0,
      endpointCount: workflow.endpoints?.length ?? 0,
    })),
    workflowRuns: (session.workflow_runs ?? []).map((run) => ({
      id: run.id,
      workflowId: run.workflow_id,
      endpointId: run.endpoint_id,
      status: run.status,
      nodeRunCount: run.node_runs?.length ?? 0,
      failureCount: run.failure_events?.length ?? 0,
      finalOutput: run.final_output ?? null,
    })),
    shell: {
      context: deps.workspaceShellContext(),
      entries: shellEntries,
      transcript: renderWorkspaceShellTranscript(shellEntries),
    },
    transcript: {
      visibleAgentId: deps.visibleTranscriptAgentId(),
      entries: deps.transcriptEntries().map(automationTranscriptEntry),
    },
    agentPanes: Object.fromEntries(
      Object.entries(deps.agentPaneEntries()).map(([agentId, entries]) => [
        agentId,
        entries.map(automationTranscriptEntry),
      ]),
    ),
    queuedPromptStrips: Object.fromEntries(
      session.agents.flatMap((agent) => {
        const items = deps.queuedPromptStripItemsForAgent?.(agent.id) ?? []
        if (items.length === 0) {
          return []
        }
        return [[agent.id, {
          selectedIndex: deps.selectedQueuedPromptIndexForAgent?.(agent.id) ?? 0,
          items: items.map(automationQueuedPromptStripItem),
        }]]
      }),
    ),
    footer: deps.footerFlash(),
  }
}

function automationQueuedPromptStripItem(item: QueuedPromptStripItem): Record<string, unknown> {
  return {
    promptId: item.promptId,
    agentId: item.agentId,
    sourceAttachmentId: item.sourceAttachmentId,
    prompt: item.prompt,
    status: item.status,
    attachmentCount: item.attachmentCount,
    steerDisabled: item.steerDisabled,
    canSteer: item.canSteer,
    canCancel: item.canCancel,
    steerDisabledReason: item.steerDisabledReason,
    cancelDisabledReason: item.cancelDisabledReason,
  }
}

function automationTranscriptEntry(entry: TranscriptEntry): Record<string, unknown> {
  const externallyObserved = sessionHistoryEntryIsExternalProviderObserved(entry)
  return {
    id: entry.id,
    role: entry.role,
    text: entry.text,
    promptId: entry.promptId ?? null,
    sourceAttachmentId: entry.sourceAttachmentId ?? null,
    attachments: entry.attachments?.map((attachment) => ({ ...attachment })) ?? null,
    queuedPrompt: entry.queuedPrompt
      ? {
        promptId: entry.queuedPrompt.promptId,
        agentId: entry.queuedPrompt.agentId,
        status: entry.queuedPrompt.status,
        attachmentCount: entry.queuedPrompt.attachmentCount,
        steerDisabled: entry.queuedPrompt.steerDisabled,
        canSteer: entry.queuedPrompt.canSteer,
        canCancel: entry.queuedPrompt.canCancel,
        steerDisabledReason: entry.queuedPrompt.steerDisabledReason,
        cancelDisabledReason: entry.queuedPrompt.cancelDisabledReason,
      }
      : null,
    source: entry.source ?? null,
    externalProvider: externallyObserved ? entry.externalProvider ?? null : null,
    externalProviderSessionId: externallyObserved ? entry.externalProviderSessionId ?? null : null,
    externalProviderTurnId: externallyObserved ? entry.externalProviderTurnId ?? null : null,
    observedAtMs: externallyObserved ? entry.observedAtMs ?? null : null,
    externalObservation: externallyObserved ? entry.externalObservation ?? null : null,
    turnId: entry.turnId ?? null,
    hidden: entry.hidden ?? false,
    blobCollapsible: entry.blobCollapsible ?? false,
    blobCollapsed: entry.blobCollapsed ?? null,
    blobTitle: entry.blobTitle ?? null,
    blobSummary: entry.blobSummary ?? null,
    historyBlobId: entry.historyBlobId ?? null,
    historyBlobAgentId: entry.historyBlobAgentId ?? null,
    historyBlobSourceId: entry.historyBlobSourceId ?? null,
    historyBlobSourceAgentId: entry.historyBlobSourceAgentId ?? null,
    historyBlobLoaded: entry.historyBlobLoaded ?? null,
    historyBlobLoading: entry.historyBlobLoading ?? null,
    historyBlobError: entry.historyBlobError ?? null,
  }
}

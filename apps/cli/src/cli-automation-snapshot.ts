import type { ShellContext } from "@arroba/kernel-client/shell-core"

import type {
  RuntimeSession,
  SliceRecord,
  TranscriptEntry,
} from "./cli-types.js"
import type { CliAutomationSnapshot } from "./cli-automation.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import { agentPaneStatusBadge } from "./split-pane-footer.js"
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
  slicesState: () => SliceRecord[]
  waitingRoomTargets: () => WaitingRoomTargetState
  themeRegistryState: () => ThemeRegistry
  selectedWorkflowId: () => string | null
  selectedWorkflowNodeId: () => string | null
  workspaceShellContext: () => ShellContext
  workspaceShellEntries: () => WorkspaceShellEntry[]
  transcriptEntries: () => TranscriptEntry[]
  agentPaneEntries: () => Record<string, TranscriptEntry[]>
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
        const badge = agentPaneStatusBadge(
          agent,
          deps.agentActivityLabels()[agent.id] ?? null,
          deps.hasPromptWorkByAgent()[agent.id] ?? false,
          agent.id === deps.streamingAgentId(),
          deps.agentBusyLatch(agent.id),
        )
        return {
          id: agent.id,
          alias: agent.alias,
          provider: agent.provider,
          state: agent.state,
          isProcessing: agent.is_processing,
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
          slices: deps.slicesState(),
        }, deps.waitingRoomTargets(), deps.themeRegistryState()).map((row) => ({
          id: row.id,
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
      visibleAgentId: deps.focusedAgentId(),
      entries: deps.transcriptEntries().map(automationTranscriptEntry),
    },
    agentPanes: Object.fromEntries(
      Object.entries(deps.agentPaneEntries()).map(([agentId, entries]) => [
        agentId,
        entries.map(automationTranscriptEntry),
      ]),
    ),
    footer: deps.footerFlash(),
  }
}

function automationTranscriptEntry(entry: TranscriptEntry): Record<string, unknown> {
  return {
    id: entry.id,
    role: entry.role,
    text: entry.text,
    queuedPrompt: entry.queuedPrompt
      ? {
        promptId: entry.queuedPrompt.promptId,
        agentId: entry.queuedPrompt.agentId,
        status: entry.queuedPrompt.status ?? null,
        steerDisabled: entry.queuedPrompt.steerDisabled ?? false,
      }
      : null,
    source: entry.source ?? null,
    externalProvider: entry.externalProvider ?? null,
    externalProviderSessionId: entry.externalProviderSessionId ?? null,
    externalProviderTurnId: entry.externalProviderTurnId ?? null,
    observedAtMs: entry.observedAtMs ?? null,
    turnId: entry.turnId ?? null,
    hidden: entry.hidden ?? false,
    blobCollapsible: entry.blobCollapsible ?? false,
    blobCollapsed: entry.blobCollapsed ?? null,
    blobTitle: entry.blobTitle ?? null,
    blobSummary: entry.blobSummary ?? null,
    historyBlobId: entry.historyBlobId ?? null,
    historyBlobAgentId: entry.historyBlobAgentId ?? null,
    historyBlobLoaded: entry.historyBlobLoaded ?? null,
    historyBlobLoading: entry.historyBlobLoading ?? null,
    historyBlobError: entry.historyBlobError ?? null,
  }
}

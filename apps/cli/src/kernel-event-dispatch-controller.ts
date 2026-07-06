import {
  normalizeRuntimeSessionWithAgentActivity,
  type RuntimeNoticeRecord,
  type RuntimeProviderRun,
  type RuntimeSession,
  type SliceRecord,
  type TerminalOutputRecord,
  type WaitingRoomPublicSessionSummary,
  type WorkflowRun,
} from "./cli-types.js"
import type { KernelEvent } from "./ipc.js"
import type { WorkflowDesignOpForwarded } from "@arroba/kernel-client/kernel-types"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { RelayStatusView } from "./relay-api.js"
import type { RemoteMachineView } from "./waiting-room-inventory-api.js"

type AssistantMessageCompletedEvent = Extract<KernelEvent, { event: "assistant_message_completed" }>
type KernelEventAgentActivityChanged = Extract<KernelEvent, { event: "agent_activity_changed" }>
type KernelEventProviderRunChanged = Extract<KernelEvent, { event: "provider_run_changed" }>
type KernelEventProviderCatalogChanged = Extract<KernelEvent, { event: "provider_catalog_changed" }>
type KernelEventRelayStatusChanged = Extract<KernelEvent, { event: "relay_status_changed" }>
type KernelEventRemoteMachinesChanged = Extract<KernelEvent, { event: "remote_machines_changed" }>
type KernelEventSessionMetadataChanged = Extract<KernelEvent, { event: "session_metadata_changed" }>
type KernelEventRuntimeInteractionsChanged = Extract<KernelEvent, { event: "runtime_interactions_changed" }>
type KernelEventSessionSnapshot = Extract<KernelEvent, { event: "session_snapshot" }>
type KernelEventSlicesChanged = Extract<KernelEvent, { event: "slices_changed" }>
type KernelEventWaitingRoomRowsChanged = Extract<KernelEvent, { event: "waiting_room_rows_changed" }>
type KernelEventWorkflowDesignOp = Extract<KernelEvent, { event: "workflow_design_op" }>
type KernelEventWorkflowRunUpdated = Extract<KernelEvent, { event: "workflow_run_updated" }>

type KernelEventDispatchControllerDeps = {
  recordDaemonActivity: (activityType: string) => void
  queueTerminalOutputRecords: (records: TerminalOutputRecord[]) => void
  applyRuntimeNotices: (notices: RuntimeNoticeRecord[]) => void
  applyAssistantMessageCompleted: (event: AssistantMessageCompletedEvent) => void
  applyKernelSessionSnapshot: (
    session: RuntimeSession,
    providerRun: RuntimeProviderRun | null,
  ) => Promise<void> | void
  applyAgentActivityChanged: (
    sessionId: string,
    agentActivity: Record<string, unknown>,
    agentActivityRevision: number | null,
  ) => Promise<void> | void
  applyProviderRunChanged: (
    sessionId: string,
    providerRun: RuntimeProviderRun | null,
  ) => Promise<void> | void
  applySessionMetadataChanged: (
    sessionId: string,
    metadata: Record<string, unknown>,
  ) => Promise<void> | void
  applyRuntimeInteractionsChanged: (
    sessionId: string,
    activeInteractions: Record<string, unknown>[],
  ) => Promise<void> | void
  applyWorkflowRunUpdated: (
    sessionId: string,
    workflowRun: WorkflowRun,
  ) => Promise<void> | void
  applyWorkflowDesignOp: (event: WorkflowDesignOpForwarded) => Promise<void> | void
  scheduleSharedPromptInputHistoryRefresh: () => void
  handleKernelSessionUnavailable: (message: string) => Promise<void> | void
  refreshWaitingRoomData: () => Promise<unknown> | unknown
  applyWaitingRoomRowsChanged: (patch: {
    inventoryVersion: string
    sessions: WaitingRoomPublicSessionSummary[]
    removedSessionIds: string[]
  }) => Promise<unknown> | unknown
  applyRelayStatusChanged: (status: RelayStatusView) => Promise<unknown> | unknown
  applyRemoteMachinesChanged: (machines: RemoteMachineView[]) => Promise<unknown> | unknown
  applyProviderCatalogChanged: (catalog: ProviderCatalog) => Promise<unknown> | unknown
  applySlicesChanged: (slices: SliceRecord[]) => Promise<unknown> | unknown
  applyTransportResumed: () => void
  resyncAttachedKernelState: (reason: string) => Promise<unknown> | unknown
  appendNotice: (message: string, tone?: "warning" | "muted") => void
  flashFooter: (message: string, tone: "info" | "error") => void
  applyTransportClosed: (message: string) => void
  recoverAttachedSessionAfterKernelRestart: () => Promise<unknown> | unknown
}

export function createKernelEventDispatchController(
  deps: KernelEventDispatchControllerDeps,
) {
  const applySessionSnapshot = async (event: KernelEventSessionSnapshot) => {
    deps.recordDaemonActivity("kernel_session_snapshot")
    deps.scheduleSharedPromptInputHistoryRefresh()
    await deps.applyKernelSessionSnapshot(
      normalizeRuntimeSessionWithAgentActivity({
        session: event.session as RuntimeSession,
        agent_activity: isRecord(event.agent_activity)
          ? event.agent_activity as RuntimeSession["agent_activity"]
          : null,
        agent_activity_revision: typeof event.agent_activity_revision === "number"
          ? event.agent_activity_revision
          : null,
      }),
      (event.provider_run as RuntimeProviderRun | null) ?? null,
    )
  }

  const handleKernelEvent = async (event: KernelEvent) => {
    switch (event.event) {
      case "terminal_output":
        deps.recordDaemonActivity("kernel_terminal_output")
        deps.queueTerminalOutputRecords(event.records as TerminalOutputRecord[])
        return
      case "runtime_notices":
        deps.applyRuntimeNotices(event.notices as RuntimeNoticeRecord[])
        return
      case "assistant_message_completed":
        deps.applyAssistantMessageCompleted(event)
        return
      case "session_snapshot":
        await applySessionSnapshot(event)
        return
      case "agent_activity_changed":
        deps.recordDaemonActivity("kernel_agent_activity_changed")
        await applyAgentActivityChanged(event)
        return
      case "provider_run_changed":
        deps.recordDaemonActivity("kernel_provider_run_changed")
        await applyProviderRunChanged(event)
        return
      case "provider_catalog_changed":
        deps.recordDaemonActivity("kernel_provider_catalog_changed")
        await applyProviderCatalogChanged(event)
        return
      case "slices_changed":
        deps.recordDaemonActivity("kernel_slices_changed")
        await applySlicesChanged(event)
        return
      case "session_metadata_changed":
        deps.recordDaemonActivity("kernel_session_metadata_changed")
        await applySessionMetadataChanged(event)
        return
      case "runtime_interactions_changed":
        deps.recordDaemonActivity("kernel_runtime_interactions_changed")
        await applyRuntimeInteractionsChanged(event)
        return
      case "heartbeat":
        deps.recordDaemonActivity("kernel_heartbeat")
        return
      case "session_unavailable":
        await deps.handleKernelSessionUnavailable(event.message)
        return
      case "waiting_room_inventory_changed":
        void deps.refreshWaitingRoomData()
        return
      case "relay_status_changed":
        deps.recordDaemonActivity("kernel_relay_status_changed")
        await applyRelayStatusChanged(event)
        return
      case "remote_machines_changed":
        deps.recordDaemonActivity("kernel_remote_machines_changed")
        await applyRemoteMachinesChanged(event)
        return
      case "waiting_room_rows_changed":
        await applyWaitingRoomRowsChanged(event)
        return
      case "workflow_design_op":
        deps.recordDaemonActivity("kernel_workflow_design_op")
        await applyWorkflowDesignOp(event)
        return
      case "workflow_run_updated":
        deps.recordDaemonActivity("kernel_workflow_run_updated")
        await applyWorkflowRunUpdated(event)
        return
      case "transport_resumed":
        deps.applyTransportResumed()
        return
      case "replay_gap":
        deps.recordDaemonActivity("kernel_replay_gap")
        deps.appendNotice("Missed retained kernel events, refreshed session state.", "warning")
        deps.flashFooter("Missed retained kernel events, refreshed session state.", "info")
        void deps.resyncAttachedKernelState("replay_gap")
        return
      case "transport_closed":
        deps.applyTransportClosed(event.message)
        void deps.recoverAttachedSessionAfterKernelRestart()
        return
    }
  }

  return {
    handleKernelEvent,
  }

  async function applyAgentActivityChanged(event: KernelEventAgentActivityChanged) {
    if (!event.session_id) {
      deps.appendNotice("Kernel sent agent activity without a session id.", "warning")
      return
    }
    if (!isRecord(event.agent_activity)) {
      deps.appendNotice(`Kernel sent malformed agent activity for session ${event.session_id}.`, "warning")
      return
    }
    await deps.applyAgentActivityChanged(
      event.session_id,
      event.agent_activity,
      typeof event.agent_activity_revision === "number"
        ? event.agent_activity_revision
        : null,
    )
  }

  async function applyProviderRunChanged(event: KernelEventProviderRunChanged) {
    if (!event.session_id) {
      deps.appendNotice("Kernel sent provider run update without a session id.", "warning")
      return
    }
    const providerRun = event.provider_run
    if (providerRun !== null && !isRecord(providerRun)) {
      deps.appendNotice(`Kernel sent malformed provider run update for session ${event.session_id}.`, "warning")
      return
    }
    await deps.applyProviderRunChanged(event.session_id, providerRun as RuntimeProviderRun | null)
  }

  async function applySessionMetadataChanged(event: KernelEventSessionMetadataChanged) {
    if (!event.session_id) {
      deps.appendNotice("Kernel sent session metadata without a session id.", "warning")
      return
    }
    if (!isRecord(event.metadata)) {
      deps.appendNotice(`Kernel sent malformed metadata for session ${event.session_id}.`, "warning")
      return
    }
    await deps.applySessionMetadataChanged(event.session_id, event.metadata)
  }

  async function applyRuntimeInteractionsChanged(event: KernelEventRuntimeInteractionsChanged) {
    if (!event.session_id) {
      deps.appendNotice("Kernel sent runtime interactions without a session id.", "warning")
      return
    }
    if (!Array.isArray(event.active_interactions) || event.active_interactions.some((interaction) => !isRecord(interaction))) {
      deps.appendNotice(`Kernel sent malformed runtime interactions for session ${event.session_id}.`, "warning")
      return
    }
    await deps.applyRuntimeInteractionsChanged(event.session_id, event.active_interactions)
  }

  async function applyWorkflowRunUpdated(event: KernelEventWorkflowRunUpdated) {
    if (!event.session_id) {
      deps.appendNotice("Kernel sent workflow run update without a session id.", "warning")
      return
    }
    if (!isRecord(event.workflow_run) || typeof event.workflow_run.id !== "string") {
      deps.appendNotice(`Kernel sent malformed workflow run update for session ${event.session_id}.`, "warning")
      return
    }
    await deps.applyWorkflowRunUpdated(event.session_id, event.workflow_run as unknown as WorkflowRun)
  }

  async function applyWorkflowDesignOp(event: KernelEventWorkflowDesignOp) {
    const designOp = event.design_op
    if (!isRecord(designOp) || typeof designOp.session_id !== "string" || !isRecord(designOp.op)) {
      deps.appendNotice("Kernel sent malformed workflow design update.", "warning")
      return
    }
    await deps.applyWorkflowDesignOp(designOp as unknown as WorkflowDesignOpForwarded)
  }

  async function applyWaitingRoomRowsChanged(event: KernelEventWaitingRoomRowsChanged) {
    if (!event.inventory_version) {
      deps.appendNotice("Kernel sent waiting-room row update without an inventory version.", "warning")
      return
    }
    if (!Array.isArray(event.sessions) || event.sessions.some((session) => !isRecord(session) || typeof session.id !== "string")) {
      deps.appendNotice("Kernel sent malformed waiting-room row update.", "warning")
      return
    }
    if (!Array.isArray(event.removed_session_ids) || event.removed_session_ids.some((sessionId) => typeof sessionId !== "string")) {
      deps.appendNotice("Kernel sent malformed waiting-room removed-session update.", "warning")
      return
    }
    await deps.applyWaitingRoomRowsChanged({
      inventoryVersion: event.inventory_version,
      sessions: event.sessions as unknown as WaitingRoomPublicSessionSummary[],
      removedSessionIds: event.removed_session_ids,
    })
  }

  async function applyRelayStatusChanged(event: KernelEventRelayStatusChanged) {
    if (!isRecord(event.status)) {
      deps.appendNotice("Kernel sent malformed relay status update.", "warning")
      return
    }
    if (typeof event.status.configured !== "boolean" || typeof event.status.connected !== "boolean") {
      deps.appendNotice("Kernel sent malformed relay status flags.", "warning")
      return
    }
    await deps.applyRelayStatusChanged(event.status as unknown as RelayStatusView)
  }

  async function applyRemoteMachinesChanged(event: KernelEventRemoteMachinesChanged) {
    if (!Array.isArray(event.machines) || event.machines.some((machine) => !isRemoteMachineRecord(machine))) {
      deps.appendNotice("Kernel sent malformed remote-machine update.", "warning")
      return
    }
    await deps.applyRemoteMachinesChanged(event.machines as unknown as RemoteMachineView[])
  }

  async function applyProviderCatalogChanged(event: KernelEventProviderCatalogChanged) {
    if (!isRecord(event.catalog) || !Array.isArray(event.catalog.all) || !isRecord(event.catalog.default) || !Array.isArray(event.catalog.connected)) {
      deps.appendNotice("Kernel sent malformed provider catalog update.", "warning")
      return
    }
    await deps.applyProviderCatalogChanged(event.catalog as unknown as ProviderCatalog)
  }

  async function applySlicesChanged(event: KernelEventSlicesChanged) {
    if (!Array.isArray(event.slices) || event.slices.some((slice) => !isRecord(slice) || typeof slice.id !== "string")) {
      deps.appendNotice("Kernel sent malformed slice-list update.", "warning")
      return
    }
    await deps.applySlicesChanged(event.slices as unknown as SliceRecord[])
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value)
}

function isRemoteMachineRecord(value: unknown): value is Record<string, unknown> {
  if (!isRecord(value)) {
    return false
  }
  const trustStatus = value.trust_status
  return typeof value.machine_id === "string"
    && typeof value.display_name === "string"
    && (trustStatus === "approved" || trustStatus === "pending" || trustStatus === "forgotten")
    && typeof value.online === "boolean"
    && typeof value.pending === "boolean"
    && typeof value.kernel_count === "number"
}

import {
  normalizeRuntimeSession,
  type RuntimeNoticeRecord,
  type RuntimeProviderRun,
  type RuntimeSession,
  type TerminalOutputRecord,
} from "./cli-types.js"
import type { KernelEvent } from "./ipc.js"

type AssistantMessageCompletedEvent = Extract<KernelEvent, { event: "assistant_message_completed" }>
type KernelEventSessionSnapshot = Extract<KernelEvent, { event: "session_snapshot" }>

type KernelEventDispatchControllerDeps = {
  recordDaemonActivity: (activityType: string) => void
  queueTerminalOutputRecords: (records: TerminalOutputRecord[]) => void
  applyRuntimeNotices: (notices: RuntimeNoticeRecord[]) => void
  applyAssistantMessageCompleted: (event: AssistantMessageCompletedEvent) => void
  applyKernelSessionSnapshot: (
    session: RuntimeSession,
    providerRun: RuntimeProviderRun | null,
  ) => Promise<void> | void
  scheduleSharedPromptInputHistoryRefresh: () => void
  handleKernelSessionUnavailable: (message: string) => Promise<void> | void
  refreshWaitingRoomData: () => Promise<unknown> | unknown
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
      normalizeRuntimeSession({
        ...(event.session as RuntimeSession),
        ...((event.agent_activity && typeof event.agent_activity === "object")
          ? { agent_activity: event.agent_activity }
          : {}),
      } as RuntimeSession),
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
      case "heartbeat":
        deps.recordDaemonActivity("kernel_heartbeat")
        deps.scheduleSharedPromptInputHistoryRefresh()
        return
      case "session_unavailable":
        await deps.handleKernelSessionUnavailable(event.message)
        return
      case "relay_status_changed":
      case "remote_machines_changed":
      case "waiting_room_inventory_changed":
        void deps.refreshWaitingRoomData()
        return
      case "workflow_design_op":
        deps.recordDaemonActivity("kernel_workflow_design_op")
        void deps.resyncAttachedKernelState("workflow_design_op")
        return
      case "workflow_run_updated":
        deps.recordDaemonActivity("kernel_workflow_run_updated")
        void deps.resyncAttachedKernelState("workflow_run_updated")
        return
      case "transport_resumed":
        deps.applyTransportResumed()
        deps.scheduleSharedPromptInputHistoryRefresh()
        void deps.resyncAttachedKernelState("transport_resumed")
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
}

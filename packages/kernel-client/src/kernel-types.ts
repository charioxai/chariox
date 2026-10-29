export const LOCAL_DAEMON_PROTOCOL_VERSION = 910

export type * from "./kernel-types-apps.js"

export type * from "./kernel-types-extensions.js"
export type * from "./kernel-types-session.js"
export type * from "./kernel-types-health.js"
export type * from "./kernel-types-resource-telemetry.js"
export type * from "./kernel-types-runtime.js"
export type * from "./kernel-types-cloud.js"
export type * from "./kernel-types-provider.js"
export type * from "./kernel-types-history.js"
export type * from "./kernel-types-workflow.js"
export type * from "./kernel-types-event-publication.js"
export type * from "./kernel-types-environment.js"
export type * from "./kernel-types-project-environment.js"

export type * from "./kernel-types-workflow-notifications.js"
export type * from "./kernel-types-project-environment-aggregate.js"
export type * from "./kernel-types-workflow-notifications.js"
export type * from "./kernel-types-kernel-browser.js"
export type * from "./kernel-types-room-terminal.js"
export { userDomainWindowBadge } from "./kernel-types-kernel-browser.js"

export * from "./notes.js"

/** MP-08 / MP-09 / MP-10 / MP-11: state changes pulled by an outside grant. */
export type EventTarget = { kernel: string; session_id: string; agent_id?: string }
export type EventSubscription = { subscription_id: string; targets: EventTarget[]; kernels: { kernel: string; online: boolean; error?: { code: string; message: string } }[]; acknowledged_cursor: number; ended: string | null }
export type OutsideEvent = {
  cursor: number; kernel: string; machine: string; session_id: string; agent_id?: string
  summary: string; at_ms: number
  refs: { task_id?: string; run_id?: string; interaction_id?: string; terminal_id?: string }
} & (
  | { type: "agent"; value: { run_state?: "idle" | "working" | "error"; task_state?: "working" | "waiting" | "blocked" | "done" | "cancelled"; reason?: string } }
  | { type: "interaction"; value: { state: "pending" | "resolved"; pointer?: { approval_id: string; kind: string; kernel: string; one_line: string; link?: string; expires_at: number } } }
  | { type: "terminal"; value: { exit_code: number | null } }
  | { type: "steer"; value: { steer_id: string } }
  | { type: "session"; value: { state: "agent_added" | "agent_removed" | "ended" } }
  | { type: "gap"; value: { kernel: string; dropped: number; from: number; to: number } }
  | { type: "kernel_health"; value: { online: boolean; reason: string } }
  | { type: "subscription_ended"; value: { reason: string } }
)
export type EventsBatch = { events: OutsideEvent[]; next_cursor: number; oldest_cursor: number; timed_out: boolean }

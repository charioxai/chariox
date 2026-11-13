import type { SessionProcessState } from "./kernel-events.js"
import type { TurnOutcome } from "./kernel-types-runtime.js"

export const LOCAL_DAEMON_PROTOCOL_VERSION = 1007

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
export type * from "./kernel-types-workflow-notifications.js"
export type * from "./kernel-types-kernel-browser.js"
export type * from "./kernel-types-room-terminal.js"
export { userDomainWindowBadge } from "./kernel-types-kernel-browser.js"

export * from "./notes.js"

/** MP-08 / MP-09 / MP-10 / MP-11: state changes pulled by an outside grant. */
export type EventTarget = { kernel: string; session_id: string; agent_id?: string }
/** A notification source the grant follows (protocol 1000). `available` is false once the source cannot emit. */
export type EventSourceFollow = { source_id: string; display_name: string; kind: "workflow_exit" | "owner_action" | "workflow_run" | "app_event"; session_id: string; subscription_id: string; available: boolean }
export type EventSubscription = { subscription_id: string; targets: EventTarget[]; sources?: EventSourceFollow[]; kernels: { kernel: string; online: boolean; error?: { code: string; message: string } }[]; acknowledged_cursor: number; ended: string | null }
/** A pending decision the holder may answer, as the person sees it (protocol 928). */
export type EventQuestion = {
  title?: string; message: string
  choices: { id: string; label: string; style?: "primary" | "secondary" | "danger" }[]
  text_reply?: { id: string; label: string; placeholder?: string; min_length: number; max_length?: number }
  expires_at_ms?: number; default_on_timeout?: string
}
export type OutsideEvent = {
  cursor: number; kernel: string; machine: string; session_id: string; agent_id?: string
  summary: string; at_ms: number
  refs: { task_id?: string; run_id?: string; interaction_id?: string; terminal_id?: string }
} & (
  | { type: "agent"; value: { run_state?: "idle" | "working" | "error"; task_state?: "working" | "waiting" | "blocked" | "done" | "cancelled"; reason?: string; prompt_id?: string; turn_id?: string; outcome?: TurnOutcome } }
  | { type: "interaction"; value: { state: "pending" | "resolved"; pointer?: { approval_id: string; kind: string; kernel: string; one_line: string; link?: string; expires_at: number }; question?: EventQuestion; resolution?: { via: "terminal" | "external_agent" | "parent_agent" | "timeout" | "cancelled"; choice_id?: string }; shown_in?: string } }
  | { type: "terminal"; value: { exit_code: number | null } }
  | { type: "process"; value: { process_id: string; state: SessionProcessState; waited_ms?: number; ended?: string } }
  | { type: "steer"; value: { steer_id: string } }
  | { type: "session"; value: { state: "agent_added" | "agent_removed" | "ended" } }
  | { type: "agent_tree"; value: { root: string; rev: number } }
  | { type: "gap"; value: { kernel: string; dropped: number; from: number; to: number } }
  | { type: "kernel_health"; value: { online: boolean; reason: string } }
  | { type: "subscription_ended"; value: { reason: string } }
  | { type: "notification"; value: { source_id: string; source_kind: "workflow_exit" | "owner_action" | "workflow_run" | "app_event"; display_name: string; occurrence_id: string; output: { message: string; artifacts?: unknown[] }; deadline_ms: number } }
)
export type EventsBatch = { events: OutsideEvent[]; next_cursor: number; oldest_cursor: number; timed_out: boolean }

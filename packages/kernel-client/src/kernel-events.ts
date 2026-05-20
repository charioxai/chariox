import type { WorkflowDesignOpForwarded } from "./kernel-types.js"

export type KernelEvent =
  | {
    event: "terminal_output"
    records: Array<Record<string, unknown>>
  }
  | {
    event: "runtime_notices"
    notices: Array<Record<string, unknown>>
  }
  | {
    event: "assistant_message_completed"
    session_id: string
    provider_run_id: string
    agent_id: string | null
    message_id: string
    completed_at_ms: number
  }
  | {
    event: "session_snapshot"
    session: Record<string, unknown>
    provider_run: Record<string, unknown> | null
    agent_activity: Record<string, unknown>
  }
  | {
    event: "session_unavailable"
    session_id: string
    message: string
  }
  | {
    event: "relay_status_changed"
    status: {
      configured: boolean
      connected: boolean
      relay_url?: string | null
      relay_token_configured: boolean
      daemon_id: string
      machine_id: string
      machine_alias?: string | null
    }
  }
  | {
    event: "remote_machines_changed"
    machines: Array<{
      machine_id: string
      machine_alias?: string | null
      registry_alias?: string | null
      display_name: string
      trust_status: "approved" | "pending" | "forgotten"
      online: boolean
      pending: boolean
      kernel_count: number
      available_providers?: string[]
    }>
  }
  | {
    event: "waiting_room_inventory_changed"
    inventory_version: string
  }
  | {
    event: "workflow_design_op"
    design_op: WorkflowDesignOpForwarded
  }
  | {
    event: "heartbeat"
    session_id: string
  }
  | {
    event: "transport_resumed"
    session_id: string
    resumed_from_event_id: number | null
  }
  | {
    event: "replay_gap"
    session_id: string
    requested_from_event_id: number
    first_retained_event_id: number | null
    latest_event_id: number | null
    message: string
  }
  | {
    event: "transport_closed"
    message: string
  }

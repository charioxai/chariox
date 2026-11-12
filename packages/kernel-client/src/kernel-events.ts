import type {
  AgentTaskExecution,
  PasskeyPrompt,
  PasskeyPromptClosure,
  ProviderAccountSummary,
  SliceRecord,
  WaitingRoomPublicSessionSummary,
  WorkflowDesignOpForwarded,
  WorkflowRun,
} from "./kernel-types.js"

export type KernelEvent =
  | { event: "session_jobs_changed"; session_id: string; jobs: JobView[] }
  | { event: "session_processes_changed"; session_id: string; processes: SessionProcess[] }
  | { event: "client_catalog_changed"; revision: number }
  | { event: "kernel_browser_frame"; subscription_id: string; frame: Record<string, unknown> }
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
    event: "agent_turn_changed"
    session_id: string
    agent_id: string
    history_revision: number
    turn: import("./kernel-types.js").SessionHistoryOutlineTurn
  }
  | {
    event: "home_extension_audit_lagged"
    session_id: string
  }
  | {
    event: "home_extension_audit_appended"
    session_id: string
    agent_id: string
    /** The row `ListHomeExtensionAudit` returns (`kind`, `timestamp_ms`, `payload`, ...). */
    audit_event: Record<string, unknown>
  }
  | {
    event: "session_snapshot"
    room_workflows?: import("./kernel-types.js").RoomWorkflowInventory
    session: Record<string, unknown>
    provider_run: Record<string, unknown> | null
    agent_activity: Record<string, unknown>
    agent_activity_revision: number
  }
  | {
    event: "agent_task_updated"
    session_id: string
    task: AgentTaskExecution
  }
  | {
    event: "room_workflows_changed"
    inventory: import("./kernel-types.js").RoomWorkflowInventory
  }
  | {
    event: "agent_activity_changed"
    session_id: string
    agent_activity: Record<string, unknown>
    agent_activity_revision: number
  }
  | {
    event: "agent_tree_changed"
    session_id: string
    root: string
    rev: number
  }
  | {
    event: "provider_run_changed"
    session_id: string
    provider_run: Record<string, unknown> | null
  }
  | {
    event: "session_metadata_changed"
    session_id: string
    metadata: Record<string, unknown>
  }
  | {
    event: "runtime_interactions_changed"
    session_id: string
    active_interactions: Array<Record<string, unknown>>
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
      display_name: string
      trust_status: "approved" | "pending" | "forgotten"
      online: boolean
      pending: boolean
      kernel_count: number
      available_providers?: string[]
      provider_accounts?: ProviderAccountSummary[]
    }>
  }
  | {
    event: "waiting_room_inventory_changed"
    inventory_version: string
  }
  | {
    event: "waiting_room_rows_changed"
    sessions_total?: number
    sessions_next_cursor?: string | null
    inventory_version: string
    structural_version: string
    activity_revision: string
    schema_version: number
    generated_at_ms: number
    launch_target?: {
      workspace_id: string
      worktree_id: string
      workspace_label?: string | null
      directory?: string | null
      worktree_label?: string | null
    } | null
    projects: import("./kernel-types-session.js").WaitingRoomPublicProjectSummary[]
    removed_project_ids: string[]
    sessions: WaitingRoomPublicSessionSummary[]
    removed_session_ids: string[]
  }
  | {
    event: "provider_catalog_changed"
    generated_at_ms: number
    catalog: Record<string, unknown>
  }
  | {
    event: "slices_changed"
    generated_at_ms: number
    slices: SliceRecord[]
  }
  | {
    event: "workflow_design_op"
    design_op: WorkflowDesignOpForwarded
  }
  | {
    event: "workflow_run_updated"
    session_id: string
    workflow_run: WorkflowRun
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
  | {
    /** Protocol 403: every passkey prompt pending for this terminal's user;
     * sent when a subscription starts and whenever the set changes. */
    event: "passkey_prompts_changed"
    /** Protocol 538: authoritative lifecycle metadata. */
    kernel_now_ms?: number
    closed?: PasskeyPromptClosure[]
    prompts: PasskeyPrompt[]
  }

export type SessionProcessWaitReason = "memory" | "disk"

/** A command that waits for room: the one row clients show, the agent reads and the room's followers get. */
export type SessionProcessWaiting = {
  reason: SessionProcessWaitReason
  /** Place in line among commands waiting for the same reason, from 1. */
  position: number
  need_bytes: number
  free_bytes: number
  command_line: string
  cwd?: string
  /** When it began to wait (Unix ms). */
  since_ms: number
}

/** Whether a command runs or waits for room; set on the command's own process. `held` has not
 * started; `paused` already started, so it never moves to another machine. */
export type SessionProcessState =
  | { kind: "running" }
  | ({ kind: "held" } & SessionProcessWaiting)
  | ({ kind: "paused" } & SessionProcessWaiting)

export type SessionProcess = {
  process_id: string
  owner: { kind: "agent" | "terminal"; id: string }
  pid: number
  command: string
  resident_set_bytes: number
  cpu_millis: number
  /** Cumulative storage reads and writes; null when the OS cannot report it. */
  disk_io_bytes: number | null
  /** Bytes this process wrote to storage; null when the OS cannot report it. */
  disk_written_bytes: number | null
  /** Bytes of deleted files it still holds open (they take space until it lets go). */
  deleted_open_bytes: number
  /** It kept running after the command that started it ended. */
  outlived: boolean
  age_ms: number
  sampled_at_ms: number
  state: SessionProcessState
  remote_available?: boolean
  remote_refusal?: "no_capacity"
  estimate?: { amount_micros: number; currency: string; kind: "total" | "per_minute"; memory_mib: number; minutes?: number }
  placement?: { machine_label: string; address: string | null }
}

export type JobView = {
  job_id: string; agent_id: string; command: string; state: "starting" | "queued" | "allocating" | "running" | "deleting" | "finished";
  placement: "local" | "remote"; started_at_ms: number; machine_label?: string | null; address?: string | null;
  class_id?: string | null; rate_per_minute?: { amount_micros: number; currency: string } | null;
  estimate_micros?: number | null; deleted: boolean; result?: JobResult | null;
}
export type JobResult = {
  exit_code: number | null; output_tail: string; log: string; files_back: string[]; conflicts: string[];
  ran_on: { machine: string; ip: string | null }; duration_ms: number;
  outcome: "completed" | "time_limit" | "memory_limit" | "outside_end" | "machine_lost" | "capacity_unavailable" | "cancelled";
}

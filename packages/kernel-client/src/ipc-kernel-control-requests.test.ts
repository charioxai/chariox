import assert from "node:assert/strict"
import test from "node:test"

import { getDaemonHealthRequest } from "./ipc-kernel-control-requests.js"
import type { DaemonHealthProjection, DaemonHealthResponse } from "./kernel-types.js"

test("getDaemonHealthRequest has typed workspace live sync health projection", () => {
  assert.deepEqual(getDaemonHealthRequest(), { GetDaemonHealth: null })

  const projection = {
    metadata: { projection_version: 1, last_event_id: 7, generated_at_ms: 100 },
    session_command_lanes: [{ lane_id: "session-1", queue_limit: 128, queued_commands: 0 }],
    agent_command_lanes: [],
    workflow_command_lanes: [],
    provider_runtime_lanes: [],
    provider_run_actor: { enqueued_commands: 1, enqueue_rejections: 0 },
    process: { process_id: 1234, peak_resident_set_bytes: 268435456 },
    capability_executor: {
      max_concurrent_jobs: 64,
      available_permits: 63,
      submitted_jobs: 3,
      running_jobs: 1,
      completed_jobs: 2,
      failed_jobs: 0,
      rejected_jobs: 0,
      join_errors: 0,
    },
    session_projection: {
      projected_sessions: 1,
      projected_session_list_entries: 1,
      active_prompts: 1,
      queued_prompts: 0,
    },
    agent_runtime_projection: { projected_agents: 1, active_prompts: 1, queued_prompts: 0 },
    provider_catalog: { cached: true, expired: false, age_ms: 12, ttl_ms: 5_000 },
    provider_runs: {
      projected_runs: 1,
      active_runs: 1,
      arroba_active_runs: 1,
      native_tui_active_runs: 0,
      duplicate_arroba_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
    transport: {
      active_connections: 1,
      active_subscriptions: 1,
      retained_event_limit: 256,
      command_result_cache_limit: 512,
      inbound_request_limit: 8,
      incoming_requests: 9,
      emitted_events: 10,
      replay_gaps: 0,
      inbound_overload_rejections: 0,
      duplicate_command_conflicts: 0,
      outgoing_queue_overflows: 0,
      slow_consumer_closes: 0,
    },
    terminal_stream: {
      pending_output_records: 1,
      pending_notice_records: 0,
      pending_completion_records: 0,
      pending_output_record_limit_per_attachment: 4096,
      trimmed_pending_output_recipients: 0,
    },
    slice_lifecycle: {
      total_slices: 2,
      running_slices: 1,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 1,
      unhealthy_slices: 0,
      attached_agents: 1,
      failed_operations: 0,
      in_progress_operations: 0,
    },
    remote_extension_sync: {
      remote_agents: 2,
      home_proxy_agents: 1,
      home_proxy_grants: 2,
      manifest_missing_agents: 0,
      synced_agents: 1,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
    },
    workspace_coordination: {
      active_worktree_claims: [
        { workspace_id: "workspace-1", worktree_id: "/repo", session_ids: ["session-1"] },
      ],
      worktree_collisions: [],
      active_operation_claims: [
        {
          claim_id: "claim-1",
          workspace_id: "workspace-1",
          worktree_id: "/repo",
          session_id: "session-1",
          attachment_id: "attachment-1",
          operation: "file_edit",
          mode: "write",
        },
      ],
    },
    workspace_live_sync: {
      active_reservations: 2,
      active_reservation_artifacts: 1,
      workspace_identity: {
        tracked_provider_runs: 3,
        identity_changed_provider_runs: 1,
        invalid_provider_runs: 1,
        current_generation_total: 2,
      },
      external_changes: {
        tracked_artifacts: 4,
        externally_changed_artifacts: 1,
        external_change_events: 5,
        live_watcher_started: true,
        live_watcher_scans: 6,
        live_watcher_scan_errors: 0,
      },
    },
    projection_invariants: { checked_sessions: 1, checked_agents: 1, mismatches: [] },
  } satisfies DaemonHealthProjection

  const response = { DaemonHealth: { projection } } satisfies DaemonHealthResponse
  assert.equal(response.DaemonHealth.projection.workspace_live_sync.active_reservations, 2)
  assert.equal(response.DaemonHealth.projection.provider_runs.arroba_active_runs, 1)
  assert.equal(response.DaemonHealth.projection.process.peak_resident_set_bytes, 268435456)
  assert.equal(response.DaemonHealth.projection.slice_lifecycle.running_slices, 1)
  assert.equal(response.DaemonHealth.projection.remote_extension_sync.home_proxy_agents, 1)
  assert.equal(response.DaemonHealth.projection.workspace_coordination.active_operation_claims[0]?.mode, "write")
})

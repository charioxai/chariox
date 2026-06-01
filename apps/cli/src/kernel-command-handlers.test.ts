import assert from "node:assert/strict"
import test from "node:test"

import type { DaemonHealthProjection } from "@arroba/kernel-client"
import {
  formatKernelHealth,
  handleKernelSlashCommand,
  kernelHealthIssueCount,
} from "./kernel-command-handlers.js"
import { makeSession } from "./command-actions-test-support.js"

function health(overrides: Partial<DaemonHealthProjection> = {}): DaemonHealthProjection {
  return {
    metadata: { projection_version: 1, last_event_id: 7, generated_at_ms: 100 },
    session_command_lanes: [],
    agent_command_lanes: [],
    workflow_command_lanes: [],
    provider_runtime_lanes: [],
    provider_run_actor: { enqueued_commands: 0, enqueue_rejections: 0 },
    process: { process_id: 1234, peak_resident_set_bytes: 268435456 },
    capability_executor: {
      max_concurrent_jobs: 64,
      available_permits: 64,
      submitted_jobs: 0,
      running_jobs: 0,
      completed_jobs: 0,
      failed_jobs: 0,
      rejected_jobs: 0,
      join_errors: 0,
    },
    session_projection: {
      projected_sessions: 1,
      projected_session_list_entries: 1,
      active_prompts: 0,
      queued_prompts: 0,
    },
    agent_runtime_projection: { projected_agents: 1, active_prompts: 0, queued_prompts: 0 },
    provider_catalog: { cached: false, expired: false, age_ms: null, ttl_ms: 5000 },
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
      incoming_requests: 0,
      emitted_events: 0,
      replay_gaps: 0,
      inbound_overload_rejections: 0,
      duplicate_command_conflicts: 0,
      outgoing_queue_overflows: 0,
      slow_consumer_closes: 0,
    },
    terminal_stream: {
      pending_output_records: 0,
      pending_notice_records: 0,
      pending_completion_records: 0,
      pending_output_record_limit_per_attachment: 4096,
      trimmed_pending_output_recipients: 0,
    },
    slice_lifecycle: {
      total_slices: 0,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 0,
      failed_operations: 0,
      in_progress_operations: 0,
    },
    workspace_coordination: {
      active_worktree_claims: [],
      worktree_collisions: [],
      active_operation_claims: [],
    },
    workspace_live_sync: {
      active_reservations: 0,
      active_reservation_artifacts: 0,
      workspace_identity: {
        tracked_provider_runs: 0,
        identity_changed_provider_runs: 0,
        invalid_provider_runs: 0,
        current_generation_total: 0,
      },
      external_changes: {
        tracked_artifacts: 0,
        externally_changed_artifacts: 0,
        external_change_events: 0,
        live_watcher_started: false,
        live_watcher_scans: 0,
        live_watcher_scan_errors: 0,
      },
    },
    projection_invariants: { checked_sessions: 1, checked_agents: 1, mismatches: [] },
    ...overrides,
  }
}

test("kernel health formatter renders provider-run invariants", () => {
  const rendered = formatKernelHealth(health())

  assert.match(rendered, /process: pid=1234 peak_rss=256.0MiB/)
  assert.match(rendered, /provider runs: projected=1 active=1 arroba=1 native_tui=0/)
  assert.match(rendered, /capabilities: running=0\/64 submitted=0 failed=0 rejected=0 join_errors=0/)
  assert.match(rendered, /transport: connections=1 subscriptions=1 incoming=0 emitted=0 replay_gaps=0 overloads=0 duplicate_commands=0 outgoing_overflows=0 slow_consumers=0/)
  assert.match(rendered, /terminal stream: pending_output=0 pending_notices=0 pending_completions=0 trimmed_recipients=0 limit=4096/)
  assert.match(rendered, /slices: total=0 running=0 starting=0 stopping=0 stopped=0 unhealthy=0 agents=0 failed_ops=0 in_progress_ops=0/)
  assert.match(rendered, /workspace coordination: claims=0 collisions=0 active_ops=0/)
  assert.match(rendered, /workspace live sync: reservations=0 artifacts=0 tracked_runs=0 identity_changed=0 invalid_runs=0/)
  assert.match(rendered, /workspace watcher: tracked=0 external_changes=0 events=0 scans=0 scan_errors=0 started=no/)
  assert.match(rendered, /provider run bindings: ok/)
  assert.match(rendered, /projection invariants: ok/)
  assert.equal(kernelHealthIssueCount(health()), 0)
})

test("kernel health formatter reports slice lifecycle issues", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 3,
      running_slices: 1,
      starting_slices: 0,
      stopping_slices: 1,
      stopped_slices: 0,
      unhealthy_slices: 1,
      attached_agents: 2,
      failed_operations: 1,
      in_progress_operations: 1,
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 2)
  assert.match(rendered, /slices: total=3 running=1 starting=0 stopping=1 stopped=0 unhealthy=1 agents=2 failed_ops=1 in_progress_ops=1/)
  assert.match(rendered, /slice lifecycle issues: unhealthy=1 failed_ops=1/)
})

test("kernel health formatter reports workspace live sync and collision issues", () => {
  const unhealthy = health({
    workspace_coordination: {
      active_worktree_claims: [
        { workspace_id: "workspace-1", worktree_id: "/repo", session_ids: ["session-1"] },
      ],
      worktree_collisions: [
        { workspace_id: "workspace-1", worktree_id: "/repo", session_ids: ["session-1", "session-2"] },
      ],
      active_operation_claims: [
        {
          claim_id: "claim-1",
          workspace_id: "workspace-1",
          worktree_id: "/repo",
          session_id: "session-1",
          attachment_id: null,
          operation: "live_sync_apply",
          mode: "write",
        },
      ],
    },
    workspace_live_sync: {
      active_reservations: 2,
      active_reservation_artifacts: 3,
      workspace_identity: {
        tracked_provider_runs: 4,
        identity_changed_provider_runs: 1,
        invalid_provider_runs: 2,
        current_generation_total: 7,
      },
      external_changes: {
        tracked_artifacts: 5,
        externally_changed_artifacts: 1,
        external_change_events: 6,
        live_watcher_started: true,
        live_watcher_scans: 8,
        live_watcher_scan_errors: 1,
      },
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 5)
  assert.match(rendered, /workspace coordination: claims=1 collisions=1 active_ops=1/)
  assert.match(rendered, /workspace live sync: reservations=2 artifacts=3 tracked_runs=4 identity_changed=1 invalid_runs=2/)
  assert.match(rendered, /workspace watcher: tracked=5 external_changes=1 events=6 scans=8 scan_errors=1 started=yes/)
  assert.match(rendered, /workspace worktree collisions:/)
  assert.match(rendered, /workspace=workspace-1 worktree=\/repo sessions=session-1,session-2/)
  assert.match(rendered, /workspace identity issues: changed=1 invalid=2/)
  assert.match(rendered, /workspace watcher scan errors: 1/)
})

test("kernel health formatter reports transport terminal and capability issues", () => {
  const unhealthy = health({
    capability_executor: {
      max_concurrent_jobs: 64,
      available_permits: 60,
      submitted_jobs: 8,
      running_jobs: 4,
      completed_jobs: 2,
      failed_jobs: 1,
      rejected_jobs: 2,
      join_errors: 1,
    },
    transport: {
      active_connections: 2,
      active_subscriptions: 3,
      retained_event_limit: 256,
      command_result_cache_limit: 512,
      inbound_request_limit: 8,
      incoming_requests: 9,
      emitted_events: 10,
      replay_gaps: 1,
      inbound_overload_rejections: 2,
      duplicate_command_conflicts: 1,
      outgoing_queue_overflows: 1,
      slow_consumer_closes: 2,
    },
    terminal_stream: {
      pending_output_records: 4,
      pending_notice_records: 3,
      pending_completion_records: 2,
      pending_output_record_limit_per_attachment: 4096,
      trimmed_pending_output_recipients: 2,
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 12)
  assert.match(rendered, /capabilities: running=4\/64 submitted=8 failed=1 rejected=2 join_errors=1/)
  assert.match(rendered, /transport: connections=2 subscriptions=3 incoming=9 emitted=10 replay_gaps=1 overloads=2 duplicate_commands=1 outgoing_overflows=1 slow_consumers=2/)
  assert.match(rendered, /terminal stream: pending_output=4 pending_notices=3 pending_completions=2 trimmed_recipients=2 limit=4096/)
  assert.match(rendered, /capability executor issues: rejected=2 join_errors=1/)
  assert.match(rendered, /transport issues: replay_gaps=1 overloads=2 duplicate_commands=1 outgoing_overflows=1 slow_consumers=2/)
  assert.match(rendered, /terminal stream trimmed pending output for 2 recipients/)
})

test("kernel health command reports duplicate provider-run bindings", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      arroba_active_runs: 2,
      native_tui_active_runs: 0,
      duplicate_arroba_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["provider-run-1", "provider-run-2"],
      }],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
  })

  await handleKernelSlashCommand({
    isAttached: () => true,
    sessionState: () => makeSession(),
    appendNotice: (message) => { notices.push(message) },
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
    getDaemonHealth: async () => unhealthy,
    transitionToNoSession: () => {},
  }, { kind: "kernel", raw: "/kernel health", args: ["health"] })

  assert.match(notices.at(-1) ?? "", /duplicate Arroba provider run bindings/)
  assert.match(notices.at(-1) ?? "", /provider-run-1,provider-run-2/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

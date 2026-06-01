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

  assert.match(rendered, /provider runs: projected=1 active=1 arroba=1 native_tui=0/)
  assert.match(rendered, /provider run bindings: ok/)
  assert.match(rendered, /projection invariants: ok/)
  assert.equal(kernelHealthIssueCount(health()), 0)
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

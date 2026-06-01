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
    session_command_lanes: [{ lane_id: "session-1", queue_limit: 128, queued_commands: 1 }],
    agent_command_lanes: [{ lane_id: "agent-1", queue_limit: 128, queued_commands: 2 }],
    workflow_command_lanes: [],
    provider_runtime_lanes: [],
    provider_run_actor: { enqueued_commands: 3, enqueue_rejections: 0 },
    process: { process_id: 1234, current_resident_set_bytes: 134217728, peak_resident_set_bytes: 268435456 },
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
      multi_interface_agent_bindings: [],
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
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
    remote_execution: {
      remote_agents: 0,
      active_remote_agents: 0,
      missing_active_worker_runs: 0,
      malformed_bindings: 0,
      issues: [],
    },
    remote_extension_sync: {
      remote_agents: 0,
      home_proxy_agents: 0,
      home_proxy_grants: 0,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      issues: [],
    },
    workspace_coordination: {
      active_worktree_claims: [],
      worktree_collisions: [],
      active_operation_claims: [],
    },
    workspace_live_sync: {
      active_reservations: 0,
      active_reservation_artifacts: 0,
      managed_mode: {
        write_fence_supported: true,
        write_fence_backend: "macos-seatbelt",
        unavailable_reason: null,
      },
      workspace_identity: {
        tracked_provider_runs: 0,
        identity_changed_provider_runs: 0,
        invalid_provider_runs: 0,
        current_generation_total: 0,
        issues: [],
      },
      external_changes: {
        tracked_artifacts: 0,
        externally_changed_artifacts: 0,
        external_change_events: 0,
        live_watcher_started: false,
        live_watcher_scans: 0,
        live_watcher_scan_errors: 0,
        issues: [],
      },
    },
    projection_invariants: { checked_sessions: 1, checked_agents: 1, mismatches: [] },
    ...overrides,
  }
}

test("kernel health formatter renders provider-run invariants", () => {
  const rendered = formatKernelHealth(health())

  assert.match(rendered, /command lanes: session=1\/1 agent=1\/2 workflow=0\/0 provider=0\/0 saturated=0/)
  assert.match(rendered, /process: pid=1234 rss=128.0MiB peak_rss=256.0MiB/)
  assert.match(rendered, /provider catalog: cached=no expired=no age=unknown ttl=5.00s/)
  assert.match(rendered, /provider runs: projected=1 active=1 arroba=1 native_tui=0/)
  assert.match(rendered, /provider run actor: enqueued=3 rejected=0/)
  assert.match(rendered, /capabilities: running=0\/64 submitted=0 failed=0 rejected=0 join_errors=0/)
  assert.match(rendered, /transport: connections=1 subscriptions=1 incoming=0 emitted=0 replay_gaps=0 overloads=0 duplicate_commands=0 outgoing_overflows=0 slow_consumers=0/)
  assert.match(rendered, /terminal stream: pending_output=0 pending_notices=0 pending_completions=0 trimmed_recipients=0 limit=4096/)
  assert.match(rendered, /slices: total=0 running=0 starting=0 stopping=0 stopped=0 unhealthy=0 agents=0 failed_ops=0 in_progress_ops=0 auth_missing=0 auth_unconfigured=0/)
  assert.match(rendered, /remote execution: remote_agents=0 active=0 missing_worker_runs=0 malformed=0/)
  assert.match(rendered, /remote extensions: remote_agents=0 home_proxy_agents=0 grants=0 synced=0 syncing=0 pending=0 failed=0 stale=0 missing=0 pending_revoke=0/)
  assert.match(rendered, /workspace coordination: claims=0 collisions=0 active_ops=0/)
  assert.match(rendered, /workspace live sync: reservations=0 artifacts=0 managed_write_fence=yes backend=macos-seatbelt tracked_runs=0 identity_changed=0 invalid_runs=0/)
  assert.match(rendered, /workspace watcher: tracked=0 external_changes=0 events=0 scans=0 scan_errors=0 started=no/)
  assert.match(rendered, /provider run bindings: ok/)
  assert.match(rendered, /projection invariants: ok/)
  assert.equal(kernelHealthIssueCount(health()), 0)
})

test("kernel health formatter reports saturated command lanes", () => {
  const unhealthy = health({
    session_command_lanes: [{ lane_id: "session-1", queue_limit: 2, queued_commands: 2 }],
    agent_command_lanes: [{ lane_id: "agent-1", queue_limit: 128, queued_commands: 3 }],
    workflow_command_lanes: [{ lane_id: "workflow-session-1", queue_limit: 1, queued_commands: 1 }],
    provider_runtime_lanes: [{ lane_id: "provider-run-1", queue_limit: 1, queued_commands: 0 }],
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 2)
  assert.match(rendered, /command lanes: session=1\/2 agent=1\/3 workflow=1\/1 provider=1\/0 saturated=2/)
  assert.match(rendered, /command lane saturation: 2 lanes at capacity/)
  assert.match(rendered, /session lane=session-1 queued=2\/2/)
  assert.match(rendered, /workflow lane=workflow-session-1 queued=1\/1/)
  assert.match(rendered, /next: wait for active operations to drain/)
})

test("kernel health formatter reports provider-run actor backpressure", () => {
  const unhealthy = health({
    provider_run_actor: { enqueued_commands: 12, enqueue_rejections: 2 },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 2)
  assert.match(rendered, /provider run actor: enqueued=12 rejected=2/)
  assert.match(rendered, /provider run actor rejected 2 commands/)
  assert.match(rendered, /next: wait for provider-run command queues to drain/)
})

test("kernel health formatter reports stale provider catalog", () => {
  const unhealthy = health({
    provider_catalog: { cached: true, expired: true, age_ms: 70_000, ttl_ms: 60_000 },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /provider catalog: cached=yes expired=yes age=1.17m ttl=1.00m/)
  assert.match(rendered, /provider catalog is stale: age=1.17m ttl=1.00m/)
  assert.match(rendered, /next: refresh provider\/model selection/)
})

test("kernel health formatter reports provider-run identity issues", () => {
  const unhealthy = health({
    provider_runs: {
      projected_runs: 4,
      active_runs: 3,
      arroba_active_runs: 2,
      native_tui_active_runs: 1,
      duplicate_arroba_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["run-1", "run-2"],
      }],
      multi_interface_agent_bindings: [{
        session_id: "session-2",
        agent_id: "agent-2",
        provider_run_ids: ["run-3:arroba", "run-4:native_tui"],
      }],
      orphaned_active_runs: [{
        provider_run_id: "run-orphan",
        session_id: "missing-session",
        agent_id: null,
        details: "provider run points at a missing session",
      }],
      session_active_run_mismatches: [{
        session_id: "session-3",
        active_provider_run_id: "run-missing",
        details: "active provider run is not projected",
      }],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 4)
  assert.match(rendered, /provider runs: projected=4 active=3 arroba=2 native_tui=1/)
  assert.match(rendered, /duplicate Arroba provider run bindings:/)
  assert.match(rendered, /session=session-1 agent=agent-1 runs=run-1,run-2/)
  assert.match(rendered, /multi-interface provider run bindings:/)
  assert.match(rendered, /session=session-2 agent=agent-2 runs=run-3:arroba,run-4:native_tui/)
  assert.match(rendered, /orphaned active provider runs:/)
  assert.match(rendered, /run=run-orphan session=missing-session agent=-: provider run points at a missing session/)
  assert.match(rendered, /session active provider run pointer issues:/)
  assert.match(rendered, /session=session-3 active=run-missing: active provider run is not projected/)
  assert.match(rendered, /next: inspect the session and relaunch the affected agent/)
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
      issues: [{
        slice_id: "slice-1",
        name: "dev",
        status: "unhealthy",
        last_operation: "start",
        last_operation_status: "failed",
        last_error: "worker kernel discovery timed out",
        session_ids: ["session-1"],
        agent_ids: ["agent-1", "agent-2"],
        worktree_id: "/repo",
      }],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /slices: total=3 running=1 starting=0 stopping=1 stopped=0 unhealthy=1 agents=2 failed_ops=1 in_progress_ops=1/)
  assert.match(rendered, /slice lifecycle issues: unhealthy=1 failed_ops=1/)
  assert.match(rendered, /slice=dev \(slice-1\) status=unhealthy op=start op_status=failed worktree=\/repo agents=agent-1,agent-2: worker kernel discovery timed out/)
  assert.match(rendered, /next: run \/slice doctor for the affected slice/)
})

test("kernel health formatter renders lifecycle issues without failed counters", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 1,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 1,
      unhealthy_slices: 0,
      attached_agents: 1,
      failed_operations: 0,
      in_progress_operations: 0,
      issues: [{
        slice_id: "slice-1",
        name: "dev",
        status: "stopped",
        last_operation: null,
        last_operation_status: null,
        last_error: null,
        session_ids: ["session-1"],
        agent_ids: ["agent-1"],
        worktree_id: "/repo",
      }],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /slice lifecycle issues: unhealthy=0 failed_ops=0/)
  assert.match(rendered, /slice=dev \(slice-1\) status=stopped worktree=\/repo agents=agent-1: stopped with attached agents/)
  assert.match(rendered, /next: run \/slice start for stopped slices or move attached agents to a running slice/)
})

test("kernel health formatter reports slice provider auth issues", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 2,
      running_slices: 2,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 2,
      failed_operations: 0,
      in_progress_operations: 0,
      issues: [],
      provider_auth_missing_slices: 1,
      provider_auth_unconfigured_slices: 1,
      provider_auth_issues: [{
        slice_id: "slice-1",
        name: "dev",
        status: "running",
        session_ids: ["session-1"],
        agent_ids: ["agent-1"],
        worktree_id: "/repo",
        provider: "codex",
        provider_auth_state: "not_configured",
        alias: "work",
        identity: "work",
        details: "slice provider account needs login or import",
      }],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /slice provider auth issues: missing=1 unconfigured=1/)
  assert.match(rendered, /slice=dev \(slice-1\) status=running worktree=\/repo agents=agent-1 provider=codex state=not_configured alias=work identity=work: slice provider account needs login or import/)
  assert.match(rendered, /next: run \/slice doctor/)
})

test("kernel health formatter reports remote execution issues", () => {
  const unhealthy = health({
    remote_execution: {
      remote_agents: 2,
      active_remote_agents: 1,
      missing_active_worker_runs: 1,
      malformed_bindings: 1,
      issues: [
        {
          kind: "missing_active_worker_provider_run",
          session_id: "session-1",
          agent_id: "agent-remote",
          agent_ref: "agent-remote",
          worker_kernel_id: "worker-kernel",
          worker_machine_id: "worker-machine",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
          active_worker_provider_run_id: null,
          state: "working",
          is_processing: true,
          worktree_id: "/repo",
          details: "active remote agent has no active worker provider run id",
        },
      ],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /remote execution: remote_agents=2 active=1 missing_worker_runs=1 malformed=1/)
  assert.match(rendered, /remote execution issues: missing_worker_runs=1 malformed=1/)
  assert.match(rendered, /agent=agent-remote \(agent-remote\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 state=working processing=yes kind=missing_active_worker_provider_run worktree=\/repo: active remote agent has no active worker provider run id/)
  assert.match(rendered, /next: run \/agent inspect agent-remote; reconnect or relaunch/)
})

test("kernel health formatter reports remote extension sync issues", () => {
  const unhealthy = health({
    remote_extension_sync: {
      remote_agents: 4,
      home_proxy_agents: 3,
      home_proxy_grants: 5,
      manifest_missing_agents: 1,
      synced_agents: 1,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 1,
      stale_agents: 1,
      pending_revoke_agents: 1,
      issues: [
        {
          session_id: "session-1",
          agent_id: "agent-failed",
          agent_ref: "agent-failed",
          worker_kernel_id: "worker-kernel",
          worker_machine_id: "worker-machine",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
          active_worker_provider_run_id: "worker-run-1",
          state: "failed",
          manifest_hash: "hash-failed",
          last_error: "relay offline",
          pending_revoke: true,
          home_proxy_grants: ["connector:status-api"],
          worktree_id: "/repo",
        },
      ],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /remote extensions: remote_agents=4 home_proxy_agents=3 grants=5 synced=1 syncing=0 pending=0 failed=1 stale=1 missing=1 pending_revoke=1/)
  assert.match(rendered, /remote extension sync issues: failed=1 stale=1 missing=1 pending_revoke=1/)
  assert.match(rendered, /agent=agent-failed \(agent-failed\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 worker_run=worker-run-1 state=failed pending_revoke=yes hash=hash-failed worktree=\/repo grants=connector:status-api: relay offline/)
  assert.match(rendered, /next: run \/extension sync-status <agent>/)
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
      managed_mode: {
        write_fence_supported: false,
        write_fence_backend: null,
        unavailable_reason: "workspace live sync managed mode needs selective write fencing",
      },
      workspace_identity: {
        tracked_provider_runs: 4,
        identity_changed_provider_runs: 1,
        invalid_provider_runs: 2,
        current_generation_total: 7,
        issues: [{
          provider_run_id: "provider-run-identity",
          root: "/repo",
          generation: 7,
          valid: false,
          baseline_fingerprint: "root-a",
          current_fingerprint: "root-b",
          baseline_branch: "main",
          current_branch: "feature",
          baseline_head_commit: "abc123",
          current_head_commit: "def456",
          baseline_repo_url: "git@example.com:repo.git",
          current_repo_url: "git@example.com:repo.git",
        }],
      },
      external_changes: {
        tracked_artifacts: 5,
        externally_changed_artifacts: 1,
        external_change_events: 6,
        live_watcher_started: true,
        live_watcher_scans: 8,
        live_watcher_scan_errors: 1,
        issues: [{
          artifact_key: "root-a:src/lib.rs",
          provider_run_id: "provider-run-external",
          workspace_fingerprint: "root-a",
          workspace_root: "/repo",
          path: "src/lib.rs",
        }],
      },
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 7)
  assert.match(rendered, /workspace coordination: claims=1 collisions=1 active_ops=1/)
  assert.match(rendered, /workspace live sync: reservations=2 artifacts=3 managed_write_fence=no backend=- tracked_runs=4 identity_changed=1 invalid_runs=2/)
  assert.match(rendered, /workspace watcher: tracked=5 external_changes=1 events=6 scans=8 scan_errors=1 started=yes/)
  assert.match(rendered, /workspace worktree collisions:/)
  assert.match(rendered, /workspace=workspace-1 worktree=\/repo sessions=session-1,session-2/)
  assert.match(rendered, /next: move one session\/agent to a different worktree/)
  assert.match(rendered, /workspace active operations:/)
  assert.match(rendered, /write live_sync_apply workspace=workspace-1 worktree=\/repo session=session-1/)
  assert.match(rendered, /next: stop and relaunch affected managed\/tracked provider runs/)
  assert.match(rendered, /next: check workspace paths and permissions/)
  assert.match(rendered, /workspace identity issues: changed=1 invalid=2/)
  assert.match(rendered, /run=provider-run-identity root=\/repo generation=7 valid=no fingerprint=root-a->root-b branch=main->feature head=abc123->def456 repo=git@example.com:repo.git->git@example.com:repo.git/)
  assert.match(rendered, /workspace external changes:/)
  assert.match(rendered, /run=provider-run-external root=\/repo path=src\/lib.rs fingerprint=root-a/)
  assert.match(rendered, /next: inspect the path; rerun or reconcile the affected managed\/tracked turn/)
  assert.match(rendered, /workspace watcher scan errors: 1/)
  assert.match(rendered, /workspace live sync managed mode unavailable: workspace live sync managed mode needs selective write fencing/)
  assert.match(rendered, /next: select tracked mode/)
})

test("kernel health counts unsupported managed workspace live sync as an issue", () => {
  const unhealthy = health({
    workspace_live_sync: {
      active_reservations: 0,
      active_reservation_artifacts: 0,
      managed_mode: {
        write_fence_supported: false,
        write_fence_backend: null,
        unavailable_reason: "managed mode needs a selective write fence",
      },
      workspace_identity: {
        tracked_provider_runs: 0,
        identity_changed_provider_runs: 0,
        invalid_provider_runs: 0,
        current_generation_total: 0,
        issues: [],
      },
      external_changes: {
        tracked_artifacts: 0,
        externally_changed_artifacts: 0,
        external_change_events: 0,
        live_watcher_started: false,
        live_watcher_scans: 0,
        live_watcher_scan_errors: 0,
        issues: [],
      },
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /workspace live sync managed mode unavailable: managed mode needs a selective write fence/)
  assert.match(rendered, /next: select tracked mode/)
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
  assert.match(rendered, /next: reconnect stale clients/)
  assert.match(rendered, /terminal stream trimmed pending output for 2 recipients/)
  assert.match(rendered, /next: refresh the terminal session/)
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
      multi_interface_agent_bindings: [],
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
  assert.match(notices.at(-1) ?? "", /next: inspect the agent and stop duplicate provider runs/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

test("kernel health command reports multi-interface provider-run bindings", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      arroba_active_runs: 1,
      native_tui_active_runs: 1,
      duplicate_arroba_agent_bindings: [],
      multi_interface_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["provider-run-1:arroba", "provider-run-2:native_tui"],
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

  assert.doesNotMatch(notices.at(-1) ?? "", /provider run bindings: ok/)
  assert.match(notices.at(-1) ?? "", /multi-interface provider run bindings/)
  assert.match(notices.at(-1) ?? "", /provider-run-1:arroba,provider-run-2:native_tui/)
  assert.match(notices.at(-1) ?? "", /close the extra native TUI or Arroba provider run/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

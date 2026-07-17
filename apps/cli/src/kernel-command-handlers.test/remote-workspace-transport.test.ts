import {
  assert,
  formatKernelHealth,
  formatKernelRemoteRuntimeHealth,
  handleKernelSlashCommand,
  health,
  kernelHealthIssueCount,
  kernelRemoteRuntimeIssueCount,
  kernelRemoteRuntimeReadiness,
  makeSession,
  test,
} from "../kernel-command-handlers.test-support.js"

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
  assert.match(rendered, /remote runtime invariants: provider_runs=ok; worker_runs=attention missing_worker_runs=1 malformed=1; slices=ok; manifests=settled; live_sync_scope=selected-workspace-only/)
  assert.match(rendered, /remote execution issues: missing_worker_runs=1 malformed=1/)
  assert.match(rendered, /agent=agent-remote \(agent-remote\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 state=working processing=yes kind=missing_active_worker_provider_run worktree=\/repo: active remote agent has no active worker provider run id/)
  assert.match(rendered, /next: run \/kernel remote-runtime; run \/agent inspect agent-remote; run \/machine kernels worker-machine; reconnect or relaunch/)
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
        {
          session_id: "session-1",
          agent_id: "agent-missing",
          agent_ref: "agent-missing",
          worker_kernel_id: "worker-kernel",
          worker_machine_id: "worker-machine",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-2",
          active_worker_provider_run_id: null,
          state: "missing",
          manifest_hash: null,
          last_error: null,
          pending_revoke: false,
          home_proxy_grants: ["mcp:github"],
          worktree_id: "/repo",
        },
      ],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 2)
  assert.match(rendered, /remote extensions: remote_agents=4 home_proxy_agents=3 grants=5 synced=1 syncing=0 pending=0 failed=1 stale=1 missing=1 pending_revoke=1/)
  assert.match(rendered, /remote extension runtime: home owns grants, credentials, and execution; workers receive projected manifests only/)
  assert.match(rendered, /remote extension sync issues: failed=1 stale=1 missing=1 pending_revoke=1/)
  assert.match(rendered, /agent=agent-failed \(agent-failed\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-1 worker_run=worker-run-1 state=failed pending_revoke=yes hash=hash-failed worktree=\/repo grants=connector:status-api: relay offline/)
  assert.match(rendered, /next: keep the home revoke in place; run \/extension sync-status agent-failed; run \/machine kernels worker-machine if the revoke stays pending; use \/extension sync-retry agent-failed after the worker reconnects/)
  assert.match(rendered, /agent=agent-missing \(agent-missing\) session=session-1 worker=worker-kernel\/worker-machine lease=lease-1 leased_agent=leased-agent-2 state=missing worktree=\/repo grants=mcp:github/)
})

test("kernel health formatter separates Worker extension failures from Home", () => {
  const unhealthy = health({
    remote_extension_sync: {
      remote_agents: 1,
      home_proxy_agents: 0,
      home_proxy_grants: 0,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      worker_extension_agents: 1,
      worker_extension_grants: 2,
      worker_manifest_missing_agents: 0,
      worker_synced_agents: 0,
      worker_syncing_agents: 0,
      worker_pending_agents: 0,
      worker_failed_agents: 1,
      worker_stale_agents: 0,
      worker_pending_revoke_agents: 1,
      issues: [{
        session_id: "session-1",
        agent_id: "agent-worker",
        agent_ref: "agent-worker",
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "worker-machine",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
        active_worker_provider_run_id: "worker-run-1",
        state: "failed",
        manifest_hash: "worker-hash",
        last_error: "worker offline",
        pending_revoke: true,
        source: "worker",
        home_proxy_grants: [],
        worker_grants: ["script:deploy", "connector:status-api"],
        worktree_id: "/repo",
      }],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /worker extensions: agents=1 grants=2 synced=0 syncing=0 pending=0 failed=1 stale=0 missing=0 pending_revoke=1/)
  assert.match(rendered, /worker extension runtime: worker owns definitions, credentials, validation, and execution; credentials stay on worker/)
  assert.match(rendered, /worker extension sync issues: failed=1 stale=0 missing=0 pending_revoke=1/)
  assert.match(rendered, /state=failed pending_revoke=yes hash=worker-hash worktree=\/repo source=worker grants=script:deploy,connector:status-api: worker offline/)
  assert.match(rendered, /next: keep the Worker revoke in place;/)
  assert.doesNotMatch(rendered, /remote extension sync issues:/)
  assert.doesNotMatch(rendered, /keep the home revoke in place/)
  assert.match(rendered, /manifests=home\(settled\) worker\(attention syncing=0 pending=0 failed=1 stale=0 missing=0 pending_revoke=1\)/)
})

test("kernel health formatter gives worker-connectivity guidance for aggregate missing manifests", () => {
  const unhealthy = health({
    remote_extension_sync: {
      remote_agents: 1,
      home_proxy_agents: 1,
      home_proxy_grants: 1,
      manifest_missing_agents: 1,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /remote extension sync issues: failed=0 stale=0 missing=1 pending_revoke=0/)
  assert.match(rendered, /next: home keeps stale home-proxy calls blocked; run \/kernel remote-runtime to identify affected agents, then use \/extension sync-status and \/extension sync-retry after worker connectivity is healthy/)
  assert.doesNotMatch(rendered, /open Extensions/)
  assert.doesNotMatch(rendered, /\/extension sync-status <agent>/)
})

test("kernel remote-runtime formatter keeps home-proxy boundary visible after grants are revoked", () => {
  const pendingRevoke = health({
    remote_extension_sync: {
      remote_agents: 1,
      home_proxy_agents: 1,
      home_proxy_grants: 0,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 0,
      pending_agents: 0,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 1,
      issues: [{
        session_id: "session-1",
        agent_id: "agent-1",
        agent_ref: "A1",
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
        active_worker_provider_run_id: "worker-run-1",
        state: "synced",
        manifest_hash: "abcdef12",
        last_error: null,
        pending_revoke: true,
        home_proxy_grants: [],
        worktree_id: "/repo",
      }],
    },
  })
  const remoteRuntime = formatKernelRemoteRuntimeHealth(pendingRevoke)
  const kernelHealth = formatKernelHealth(pendingRevoke)

  assert.match(remoteRuntime, /remote extensions: remote_agents=1 home_proxy_agents=1 grants=0/)
  assert.match(remoteRuntime, /remote extension runtime: home owns grants, credentials, and execution; workers receive projected manifests only/)
  assert.match(remoteRuntime, /pending_revoke=yes/)
  assert.match(remoteRuntime, /next: keep the home revoke in place; run \/extension sync-status A1; run \/machine kernels hetzner if the revoke stays pending; use \/extension sync-retry A1 after the worker reconnects/)
  assert.match(kernelHealth, /remote extension runtime: home owns grants, credentials, and execution; workers receive projected manifests only/)
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

  assert.equal(kernelHealthIssueCount(unhealthy), 6)
  assert.match(rendered, /workspace coordination: claims=1 collisions=1 active_ops=1/)
  assert.match(rendered, /workspace live sync: reservations=2 artifacts=3 managed_write_fence=no backend=- tracked_runs=4 identity_changed=1 invalid_runs=2/)
  assert.match(rendered, /workspace live sync scope: selected workspace\/worktree only; other repositories unrestricted; affected roots: \/repo/)
  assert.match(rendered, /live_sync_scope=attention identity_changed=1 invalid=2 external_changes=1 scan_errors=1 roots=\/repo/)
  assert.match(rendered, /workspace watcher: tracked=5 external_changes=1 events=6 scans=8 scan_errors=1 started=yes/)
  assert.match(rendered, /workspace worktree collisions:/)
  assert.match(rendered, /workspace=workspace-1 worktree=\/repo sessions=session-1,session-2/)
  assert.match(rendered, /next: run \/workspace sync targets and \/workspace sync conflicts; move one session\/agent to a different worktree/)
  assert.match(rendered, /workspace active operations:/)
  assert.match(rendered, /write live_sync_apply workspace=workspace-1 worktree=\/repo session=session-1/)
  assert.match(rendered, /next: stop and relaunch provider run provider-run-identity after confirming the selected worktree/)
  assert.match(rendered, /next: run \/workspace sync status, then \/workspace sync ignore; check \.arrobaignore, selected workspace paths, and permissions before refreshing/)
  assert.match(rendered, /workspace identity issues: changed=1 invalid=2/)
  assert.match(rendered, /run=provider-run-identity root=\/repo generation=7 valid=no fingerprint=root-a->root-b branch=main->feature head=abc123->def456 repo=git@example.com:repo.git->git@example.com:repo.git/)
  assert.match(rendered, /workspace external changes:/)
  assert.match(rendered, /run=provider-run-external root=\/repo path=src\/lib.rs fingerprint=root-a/)
  assert.match(rendered, /next: inspect path src\/lib\.rs for provider run provider-run-external; rerun or reconcile the affected managed\/tracked turn/)
  assert.match(rendered, /workspace watcher scan errors: 1/)
  assert.match(rendered, /workspace live sync managed capability: unavailable \(workspace live sync managed mode needs selective write fencing\); tracked\/off modes unaffected/)
})

test("kernel health reports unsupported managed workspace live sync as capability info", () => {
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

  assert.equal(kernelHealthIssueCount(unhealthy), 0)
  assert.match(rendered, /workspace live sync managed capability: unavailable \(managed mode needs a selective write fence\); tracked\/off modes unaffected/)
  assert.doesNotMatch(rendered, /next: select tracked mode/)
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
      relay_reconnect_attempts: 3,
      relay_last_reconnect_reason: "relay heartbeat send failed",
      relay_last_reconnect_delay_ms: 750,
      relay_last_reconnect_url: "wss://relay-b.example.test",
      relay_last_connected_url: "wss://relay-a.example.test",
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

  assert.equal(kernelHealthIssueCount(unhealthy), 15)
  assert.match(rendered, /capabilities: running=4\/64 submitted=8 failed=1 rejected=2 join_errors=1/)
  assert.match(rendered, /transport: connections=2 subscriptions=3 incoming=9 emitted=10 replay_gaps=1 overloads=2 duplicate_commands=1 outgoing_overflows=1 slow_consumers=2 relay_reconnects=3/)
  assert.match(rendered, /terminal stream: pending_output=4 pending_notices=3 pending_completions=2 trimmed_recipients=2 limit=4096/)
  assert.match(rendered, /capability executor issues: rejected=2 join_errors=1/)
  assert.match(rendered, /transport issues: replay_gaps=1 overloads=2 duplicate_commands=1 outgoing_overflows=1 slow_consumers=2 relay_reconnects=3/)
  assert.match(rendered, /relay reconnect: attempts=3 last_url=wss:\/\/relay-b\.example\.test last_delay=750ms last_reason=relay heartbeat send failed last_connected=wss:\/\/relay-a\.example\.test/)
  assert.match(rendered, /next: reconnect stale clients/)
  assert.match(rendered, /terminal stream trimmed pending output for 2 recipients/)
  assert.match(rendered, /next: refresh the terminal session/)
  assert.match(rendered, /support bundle: after reproducing, run \/kernel debug-bundle <label> from TUI or kernel debug-bundle <label> from arroba-shell/)
})

test("kernel health formatter reports session and agent projection invariants", () => {
  const unhealthy = health({
    projection_invariants: {
      checked_sessions: 1,
      checked_agents: 1,
      mismatches: [
        {
          kind: "stale_focused_agent",
          session_id: "session-1",
          agent_id: "missing-agent",
          details: "focused agent is not present in the session agent list",
        },
        {
          kind: "agent_record_not_in_session_projection",
          session_id: "session-1",
          agent_id: "agent-2",
          details: "canonical agent record is not present in its projected session agent list",
        },
      ],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 2)
  assert.match(rendered, /projection invariant mismatches:/)
  assert.match(rendered, /stale_focused_agent session=session-1 agent=missing-agent: focused agent is not present in the session agent list/)
  assert.match(rendered, /agent_record_not_in_session_projection session=session-1 agent=agent-2: canonical agent record is not present in its projected session agent list/)
  assert.match(rendered, /next: refresh the session; restart the kernel if the invariant mismatch persists/)
})

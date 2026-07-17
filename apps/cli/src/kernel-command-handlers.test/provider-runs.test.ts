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

test("kernel health formatter renders provider-run invariants", () => {
  const rendered = formatKernelHealth(health())

  assert.match(rendered, /command lanes: session=1\/1 agent=1\/2 workflow=0\/0 provider=0\/0 saturated=0/)
  assert.match(rendered, /process: pid=1234 rss=128.0MiB peak_rss=256.0MiB/)
  assert.match(rendered, /provider catalog: cached=no expired=no age=unknown ttl=5.00s/)
  assert.match(rendered, /provider runs: projected=1 active=1 arroba=1 native_tui=0/)
  assert.match(rendered, /provider run actor: enqueued=3 rejected=0/)
  assert.match(rendered, /capabilities: running=0\/64 submitted=0 failed=0 rejected=0 join_errors=0/)
  assert.match(rendered, /transport: connections=1 subscriptions=1 incoming=0 emitted=0 replay_gaps=0 overloads=0 duplicate_commands=0 outgoing_overflows=0 slow_consumers=0 relay_reconnects=0/)
  assert.match(rendered, /terminal stream: pending_output=0 pending_notices=0 pending_completions=0 trimmed_recipients=0 limit=4096/)
  assert.match(rendered, /slices: total=0 running=0 starting=0 stopping=0 stopped=0 unhealthy=0 agents=0 failed_ops=0 in_progress_ops=0 auth_missing=0 auth_unconfigured=0/)
  assert.match(rendered, /remote runtime authority: home kernel owns sessions, prompts, and live sync; Home extensions execute on home and Worker extensions execute on worker; credentials stay at their source/)
  assert.match(rendered, /remote execution: remote_agents=0 active=0 missing_worker_runs=0 malformed=0/)
  assert.match(rendered, /remote extensions: remote_agents=0 home_proxy_agents=0 grants=0 synced=0 syncing=0 pending=0 failed=0 stale=0 missing=0 pending_revoke=0/)
  assert.doesNotMatch(rendered, /remote extension runtime:/)
  assert.match(rendered, /remote runtime invariants: provider_runs=ok; worker_runs=ok; slices=ok; manifests=settled; live_sync_scope=selected-workspace-only; audit=durable-transitions-required/)
  assert.match(rendered, /remote runtime readiness: ok/)
  assert.match(rendered, /workspace coordination: claims=0 collisions=0 active_ops=0/)
  assert.match(rendered, /workspace live sync: reservations=0 artifacts=0 managed_write_fence=yes backend=macos-seatbelt tracked_runs=0 identity_changed=0 invalid_runs=0/)
  assert.match(rendered, /workspace live sync scope: selected workspace\/worktree only; other repositories unrestricted/)
  assert.match(rendered, /workspace watcher: tracked=0 external_changes=0 events=0 scans=0 scan_errors=0 started=no/)
  assert.match(rendered, /provider run invariants: ok/)
  assert.match(rendered, /projection invariants: ok/)
  assert.doesNotMatch(rendered, /support bundle:/)
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
  assert.match(rendered, /next: wait for active operations to drain; inspect session session-1, workflow workflow-session-1/)
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
      terminal_diagnostics: [],
      duplicate_arroba_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["run-1", "run-2"],
      }],
      duplicate_native_tui_agent_bindings: [],
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
  assert.doesNotMatch(rendered, /provider run invariants: ok/)
  assert.match(rendered, /invariant: normal Arroba launches should replace idle same-agent runs instead of creating duplicates/)
  assert.match(rendered, /next: run \/agent inspect agent-1; run \/provider processes; capture a debug bundle, then stop duplicate provider runs before sending prompts to that agent/)
  assert.match(rendered, /next: run \/agent inspect agent-2; run \/provider processes; close the extra native TUI or Arroba provider run before sending prompts to that agent/)
  assert.match(rendered, /next: refresh the session; stop or relaunch provider run run-orphan if it stays active/)
  assert.match(rendered, /next: inspect session session-3 and relaunch the affected agent/)
})

test("kernel health formatter reports provider-run terminal diagnostics", () => {
  const unhealthy = health({
    provider_runs: {
      projected_runs: 1,
      active_runs: 1,
      arroba_active_runs: 1,
      native_tui_active_runs: 0,
      terminal_diagnostics: [{
        provider_run_id: "run-timeout",
        session_id: "session-1",
        agent_id: "agent-1",
        provider: "codex",
        state: "Running",
        diagnostic: "provider produced no terminal output within 10m",
      }],
      duplicate_arroba_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.match(rendered, /provider run terminal diagnostics:/)
  assert.match(rendered, /run=run-timeout provider=codex state=Running session=session-1 agent=agent-1: provider produced no terminal output within 10m/)
  assert.match(rendered, /next: run \/agent inspect agent-1; run \/provider processes; relaunch agent agent-1 if the diagnostic persists; capture a debug bundle before restarting the kernel/)
  assert.match(rendered, /remote runtime invariants: provider_runs=attention duplicate=0 mixed=0 orphaned=0 pointer=0 terminal=1 actor_rejects=0/)
  assert.equal(kernelRemoteRuntimeIssueCount(unhealthy), 1)
  assert.deepEqual(kernelRemoteRuntimeReadiness(unhealthy), {
    state: "blocked",
    issueCount: 1,
    attentionCount: 1,
  })
})

test("kernel health formatter avoids placeholder agent recovery for unbound provider diagnostics", () => {
  const unhealthy = health({
    provider_runs: {
      projected_runs: 1,
      active_runs: 1,
      arroba_active_runs: 1,
      native_tui_active_runs: 0,
      terminal_diagnostics: [{
        provider_run_id: "run-timeout",
        session_id: "session-1",
        agent_id: null,
        provider: "codex",
        state: "Running",
        diagnostic: "provider produced no terminal output within 10m",
      }],
      duplicate_arroba_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.match(rendered, /provider run terminal diagnostics:/)
  assert.match(rendered, /run=run-timeout provider=codex state=Running session=session-1 agent=-: provider produced no terminal output within 10m/)
  assert.match(rendered, /next: identify the affected agent from \/provider processes or the debug bundle; run \/provider processes; relaunch provider run run-timeout if the diagnostic persists; capture a debug bundle before restarting the kernel/)
  assert.doesNotMatch(rendered, /<agent>/)
})

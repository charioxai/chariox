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

test("kernel health command reports duplicate provider-run bindings", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      chariox_active_runs: 2,
      native_tui_active_runs: 0,
      terminal_diagnostics: [],
      duplicate_chariox_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["provider-run-1", "provider-run-2"],
      }],
      duplicate_native_tui_agent_bindings: [],
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

  assert.match(notices.at(-1) ?? "", /duplicate Chariox provider run bindings/)
  assert.match(notices.at(-1) ?? "", /provider-run-1,provider-run-2/)
  assert.match(notices.at(-1) ?? "", /invariant: normal Chariox launches should replace idle same-agent runs instead of creating duplicates/)
  assert.match(notices.at(-1) ?? "", /next: run \/agent inspect agent-1; run \/provider processes; capture a debug bundle, then stop duplicate provider runs before sending prompts to that agent/)
  assert.match(notices.at(-1) ?? "", /support bundle: after reproducing, run \/kernel debug-bundle <label> from TUI or kernel debug-bundle <label> from chariox-shell/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

test("kernel health command reports duplicate native TUI provider-run bindings", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      chariox_active_runs: 0,
      native_tui_active_runs: 2,
      terminal_diagnostics: [],
      duplicate_chariox_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [{
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

  assert.match(notices.at(-1) ?? "", /duplicate native TUI provider run bindings/)
  assert.match(notices.at(-1) ?? "", /provider-run-1,provider-run-2/)
  assert.match(notices.at(-1) ?? "", /invariant: native TUI attachments should share one provider run per agent/)
  assert.match(notices.at(-1) ?? "", /next: run \/agent inspect agent-1; run \/provider processes; close duplicate native TUIs before sending prompts to that agent/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

test("kernel remote-runtime command opens health projection with remote footer", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []

  await handleKernelSlashCommand({
    isAttached: () => true,
    sessionState: () => makeSession(),
    appendNotice: (message) => { notices.push(message) },
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
    getDaemonHealth: async () => health(),
    transitionToNoSession: () => {},
  }, { kind: "kernel", raw: "/kernel remote-runtime", args: ["remote-runtime"] })

  assert.match(notices.at(-1) ?? "", /^remote runtime/)
  assert.match(notices.at(-1) ?? "", /provider runs: projected=1 active=1 chariox=1 native_tui=0/)
  assert.match(notices.at(-1) ?? "", /remote execution: remote_agents=0 active=0 missing_worker_runs=0 malformed=0/)
  assert.match(notices.at(-1) ?? "", /remote extensions: remote_agents=0 home_proxy_agents=0 grants=0 synced=0 syncing=0 pending=0 failed=0 stale=0 missing=0 pending_revoke=0/)
  assert.match(notices.at(-1) ?? "", /remote runtime invariants: provider_runs=ok; worker_runs=ok; slices=ok; manifests=settled; live_sync_scope=selected-workspace-only; audit=durable-transitions-required/)
  assert.match(notices.at(-1) ?? "", /workspace live sync:/)
  assert.deepEqual(flashes.at(-1), { message: "remote runtime: ok", tone: "info" })
})

test("kernel remote-runtime formatter treats provider-run invariants as blockers", () => {
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      chariox_active_runs: 2,
      native_tui_active_runs: 0,
      terminal_diagnostics: [],
      duplicate_chariox_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["provider-run-1", "provider-run-2"],
      }],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
  })
  const rendered = formatKernelRemoteRuntimeHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.equal(kernelRemoteRuntimeIssueCount(unhealthy), 1)
  assert.match(rendered, /^remote runtime/)
  assert.match(rendered, /workspace live sync scope: selected workspace\/worktree only; other repositories unrestricted/)
  assert.match(rendered, /provider runs: projected=2 active=2 chariox=2 native_tui=0/)
  assert.match(rendered, /provider run invariants: duplicate=1 mixed=0 orphaned=0 pointer=0 terminal=0 actor_rejects=0/)
  assert.match(rendered, /remote runtime invariants: provider_runs=attention duplicate=1 mixed=0 orphaned=0 pointer=0 terminal=0 actor_rejects=0; worker_runs=ok; slices=ok; manifests=settled; live_sync_scope=selected-workspace-only/)
  assert.match(rendered, /provider run issues: duplicate=1 mixed=0 orphaned=0 pointer=0 terminal=0 actor_rejects=0/)
  assert.match(rendered, /duplicate_chariox session=session-1 agent=agent-1 runs=provider-run-1,provider-run-2/)
  assert.match(rendered, /next: run \/agent inspect agent-1; run \/provider processes; capture a debug bundle, then stop duplicate provider runs before sending prompts to that agent/)
  assert.match(rendered, /remote runtime readiness: blocked \(1 issue, 1 attention\)/)
  assert.match(rendered, /remote runtime readiness next: run \/provider processes and \/agent inspect for the affected agent; close or relaunch duplicate, orphaned, or mismatched provider runs/)
})

test("kernel remote-runtime formatter treats session projection invariant drift as a blocker", () => {
  const unhealthy = health({
    projection_invariants: {
      checked_sessions: 1,
      checked_agents: 1,
      mismatches: [{
        kind: "agent_record_not_in_session_projection",
        session_id: "session-1",
        agent_id: "agent-2",
        details: "canonical agent record is not present in its projected session agent list",
      }],
    },
  })
  const rendered = formatKernelRemoteRuntimeHealth(unhealthy)

  assert.equal(kernelHealthIssueCount(unhealthy), 1)
  assert.equal(kernelRemoteRuntimeIssueCount(unhealthy), 1)
  assert.deepEqual(kernelRemoteRuntimeReadiness(unhealthy), {
    state: "blocked",
    issueCount: 1,
    attentionCount: 1,
  })
  assert.match(rendered, /session projection: checked_sessions=1 checked_agents=1 mismatches=1/)
  assert.match(rendered, /session projection invariant issues: mismatches=1/)
  assert.match(rendered, /agent_record_not_in_session_projection session=session-1 agent=agent-2: canonical agent record is not present in its projected session agent list/)
  assert.match(rendered, /next: refresh agent agent-2 in session session-1; run \/kernel health and \/agent list; capture a debug bundle before restarting the kernel if the mismatch persists/)
  assert.match(rendered, /remote runtime readiness: blocked \(1 issue, 1 attention\)/)
  assert.match(rendered, /remote runtime readiness next: refresh the affected session or agent projection; capture a debug bundle if the mismatch persists/)
})

test("kernel remote-runtime formatter avoids placeholder recovery targets", () => {
  const unhealthy = health({
    provider_runs: {
      projected_runs: 1,
      active_runs: 1,
      chariox_active_runs: 1,
      native_tui_active_runs: 0,
      terminal_diagnostics: [{
        provider_run_id: "provider-run-1",
        session_id: "session-1",
        agent_id: null,
        provider: "codex",
        state: "Running",
        diagnostic: "provider produced no terminal output within 10m",
      }],
      duplicate_chariox_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [],
      orphaned_active_runs: [],
      session_active_run_mismatches: [],
    },
  })
  const rendered = formatKernelRemoteRuntimeHealth(unhealthy)

  assert.match(rendered, /terminal run=provider-run-1 provider=codex state=Running session=session-1 agent=-: provider produced no terminal output within 10m/)
  assert.match(rendered, /next: identify the affected agent from \/provider processes or the debug bundle; run \/provider processes; relaunch provider run provider-run-1 if the diagnostic persists/)
  assert.doesNotMatch(rendered, /<agent>|<provider-run>|<session>/)
})

test("kernel remote-runtime formatter reports slice operations as degraded attention", () => {
  const settling = health({
    slice_lifecycle: {
      total_slices: 2,
      running_slices: 1,
      starting_slices: 1,
      stopping_slices: 1,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 2,
      failed_operations: 0,
      in_progress_operations: 2,
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelRemoteRuntimeHealth(settling)

  assert.equal(kernelRemoteRuntimeIssueCount(settling), 0)
  assert.deepEqual(kernelRemoteRuntimeReadiness(settling), {
    state: "degraded",
    issueCount: 0,
    attentionCount: 2,
  })
  assert.match(rendered, /remote runtime invariants: provider_runs=ok; worker_runs=ok; slices=attention starting=1 stopping=1 in_progress=2 unhealthy=0 failed_ops=0 auth_missing=0 auth_unconfigured=0; manifests=settled; live_sync_scope=selected-workspace-only/)
  assert.match(rendered, /slice operations settling: starting=1 stopping=1 in_progress=2/)
  assert.match(rendered, /next: wait for the slice operation to finish; run \/slice list to identify any stuck slice, then run \/slice doctor and inspect logs if it does not settle/)
  assert.match(rendered, /remote runtime readiness: degraded \(2 attention\)/)
  assert.match(rendered, /remote runtime readiness next: wait for slice operations to settle; if they remain in progress, run \/slice list, then \/slice doctor for the affected slice/)
  assert.doesNotMatch(rendered, /support bundle:/)
  assert.doesNotMatch(rendered, /<slice>/)

  const fullHealth = formatKernelHealth(settling)
  assert.match(fullHealth, /remote runtime readiness: degraded \(2 attention\)/)
  assert.match(fullHealth, /remote runtime readiness next: wait for slice operations to settle/)
  assert.doesNotMatch(fullHealth, /support bundle:/)
})

test("kernel remote-runtime formatter reports settling manifests as degraded attention", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const settling = health({
    remote_extension_sync: {
      remote_agents: 2,
      home_proxy_agents: 2,
      home_proxy_grants: 3,
      manifest_missing_agents: 0,
      synced_agents: 0,
      syncing_agents: 1,
      pending_agents: 1,
      failed_agents: 0,
      stale_agents: 0,
      pending_revoke_agents: 0,
      issues: [],
    },
  })
  const rendered = formatKernelRemoteRuntimeHealth(settling)

  assert.equal(kernelRemoteRuntimeIssueCount(settling), 0)
  assert.deepEqual(kernelRemoteRuntimeReadiness(settling), {
    state: "degraded",
    issueCount: 0,
    attentionCount: 2,
  })
  assert.match(rendered, /remote extension sync settling: syncing=1 pending=1/)
  assert.match(rendered, /remote runtime invariants: provider_runs=ok; worker_runs=ok; slices=ok; manifests=attention syncing=1 pending=1 failed=0 stale=0 missing=0 pending_revoke=0; live_sync_scope=selected-workspace-only/)
  assert.match(rendered, /remote runtime readiness: degraded \(2 attention\)/)
  assert.match(rendered, /remote runtime readiness next: run \/extension sync-status for affected agents; use \/extension sync-retry after worker connectivity is healthy/)
  assert.doesNotMatch(rendered, /support bundle:/)

  await handleKernelSlashCommand({
    isAttached: () => true,
    sessionState: () => makeSession(),
    appendNotice: (message) => { notices.push(message) },
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
    getDaemonHealth: async () => settling,
    transitionToNoSession: () => {},
  }, { kind: "kernel", raw: "/kernel remote-runtime", args: ["remote-runtime"] })

  assert.match(notices.at(-1) ?? "", /remote runtime readiness: degraded \(2 attention\)/)
  assert.match(notices.at(-1) ?? "", /remote runtime readiness next: run \/extension sync-status for affected agents/)
  assert.deepEqual(flashes.at(-1), { message: "remote runtime: degraded (2 attention)", tone: "error" })
})

test("kernel health command reports multi-interface provider-run bindings", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const unhealthy = health({
    provider_runs: {
      projected_runs: 2,
      active_runs: 2,
      chariox_active_runs: 1,
      native_tui_active_runs: 1,
      terminal_diagnostics: [],
      duplicate_chariox_agent_bindings: [],
      duplicate_native_tui_agent_bindings: [],
      multi_interface_agent_bindings: [{
        session_id: "session-1",
        agent_id: "agent-1",
        provider_run_ids: ["provider-run-1:chariox", "provider-run-2:native_tui"],
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

  assert.doesNotMatch(notices.at(-1) ?? "", /provider run invariants: ok/)
  assert.match(notices.at(-1) ?? "", /multi-interface provider run bindings/)
  assert.match(notices.at(-1) ?? "", /provider-run-1:chariox,provider-run-2:native_tui/)
  assert.match(notices.at(-1) ?? "", /next: run \/agent inspect agent-1; run \/provider processes; close the extra native TUI or Chariox provider run before sending prompts to that agent/)
  assert.deepEqual(flashes.at(-1), { message: "kernel health: 1 issue", tone: "error" })
})

test("kernel debug-bundle exports a session-scoped log bundle", async () => {
  const notices: string[] = []
  const flashes: Array<{ message: string; tone: string }> = []
  const bundleRequests: Array<{ sessionId: string; label: string | null }> = []

  await handleKernelSlashCommand({
    isAttached: () => true,
    sessionState: () => makeSession({ id: "session-1" }),
    appendNotice: (message) => { notices.push(message) },
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
    exportDebugBundle: async (sessionId, label) => {
      bundleRequests.push({ sessionId, label })
      return { bundleDir: "/kernel/debug-bundles/session-1-glitch", recordCount: 7, limit: 1000 }
    },
    transitionToNoSession: () => {},
  }, { kind: "kernel", raw: "/kernel debug-bundle glitch", args: ["debug-bundle", "glitch"] })

  assert.deepEqual(bundleRequests, [{ sessionId: "session-1", label: "glitch" }])
  assert.match(notices.at(-1) ?? "", /debug bundle: \/kernel\/debug-bundles\/session-1-glitch/)
  assert.match(notices.at(-1) ?? "", /location: kernel machine/)
  assert.match(notices.at(-1) ?? "", /session: session-1/)
  assert.match(notices.at(-1) ?? "", /records: 7\/1000/)
  assert.match(notices.at(-1) ?? "", /contents: manifest\.json, logs\.ndjson/)
  assert.deepEqual(flashes.at(-1), { message: "debug bundle exported: 7 records", tone: "info" })
})

test("kernel debug-bundle rejects detached and invalid usage", async () => {
  const flashes: Array<{ message: string; tone: string }> = []
  const deps = {
    isAttached: () => false,
    sessionState: () => makeSession({ id: "session-1" }),
    appendNotice: () => {},
    flashFooter: (message: string, tone: "info" | "error") => { flashes.push({ message, tone }) },
    exportDebugBundle: async () => {
      throw new Error("should not export bundle")
    },
    transitionToNoSession: () => {},
  }

  await handleKernelSlashCommand(deps, { kind: "kernel", raw: "/kernel debug-bundle", args: ["debug-bundle"] })
  await handleKernelSlashCommand({ ...deps, isAttached: () => true }, { kind: "kernel", raw: "/kernel debug-bundle one two", args: ["debug-bundle", "one", "two"] })

  assert.deepEqual(flashes, [
    { message: "attach to a session before exporting a debug bundle", tone: "error" },
    { message: "usage: /kernel debug-bundle [label]", tone: "error" },
  ])
})

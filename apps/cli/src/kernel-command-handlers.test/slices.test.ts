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
  assert.match(rendered, /next: run \/slice doctor slice-1, inspect \/slice logs slice-1, and check \/slice audit slice-1 before restarting or deleting the slice/)
})

test("kernel health formatter reports slice storage recovery directly", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 1,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 1,
      attached_agents: 0,
      failed_operations: 1,
      in_progress_operations: 0,
      issues: [{
        slice_id: "slice-1",
        name: "dev",
        status: "unhealthy",
        last_operation: "start",
        last_operation_status: "failed",
        last_error: "slice storage preflight failed for desktop: /home/slice has 0MiB free, needs 256MiB",
        session_ids: [],
        agent_ids: [],
        worktree_id: "/repo",
      }],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.match(rendered, /slice storage preflight failed for desktop/)
  assert.match(rendered, /next: free Docker\/Colima disk or delete unused slice containers\/volumes; then run \/slice start slice-1 or recreate the slice if startup still fails/)
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
  assert.match(rendered, /next: run \/slice start slice-1 for stopped slices or move attached agents to a running slice/)
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
  assert.match(rendered, /remote runtime invariants: provider_runs=ok; worker_runs=ok; slices=attention starting=0 stopping=0 in_progress=0 unhealthy=0 failed_ops=0 auth_missing=1 auth_unconfigured=1; manifests=settled; live_sync_scope=selected-workspace-only/)
  assert.match(rendered, /slice provider auth issues: missing=1 unconfigured=1/)
  assert.match(rendered, /slice=dev \(slice-1\) status=running worktree=\/repo agents=agent-1 provider=codex state=not_configured alias=work identity=work: slice provider account needs login or import/)
  assert.match(rendered, /next: run \/slice doctor slice-1; inspect \/slice audit slice-1; use \/slice auth login slice-1 codex or \/slice auth import slice-1 codex before sending prompts to agents in that slice/)
})

test("kernel health formatter avoids placeholder provider recovery when only aggregate slice auth counts are available", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 1,
      running_slices: 1,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 1,
      failed_operations: 0,
      in_progress_operations: 0,
      issues: [],
      provider_auth_missing_slices: 1,
      provider_auth_unconfigured_slices: 1,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.match(rendered, /slice provider auth issues: missing=1 unconfigured=1/)
  assert.match(rendered, /next: run \/slice list to identify affected slices; run \/slice doctor and inspect \/slice audit before choosing a provider account to login or import/)
  assert.doesNotMatch(rendered, /<provider>|provider-specific/)
})

test("kernel health formatter avoids placeholder slice recovery when only aggregate lifecycle counts are available", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 1,
      running_slices: 0,
      starting_slices: 0,
      stopping_slices: 0,
      stopped_slices: 0,
      unhealthy_slices: 1,
      attached_agents: 0,
      failed_operations: 1,
      in_progress_operations: 0,
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.match(rendered, /slice lifecycle issues: unhealthy=1 failed_ops=1/)
  assert.match(rendered, /next: run \/slice list to identify the affected slice, then run \/slice doctor and inspect logs\/audit before restarting or deleting it/)
  assert.doesNotMatch(rendered, /<slice>/)
})

test("kernel health formatter avoids placeholder slice recovery for aggregate settling operations", () => {
  const unhealthy = health({
    slice_lifecycle: {
      total_slices: 2,
      running_slices: 1,
      starting_slices: 1,
      stopping_slices: 1,
      stopped_slices: 0,
      unhealthy_slices: 0,
      attached_agents: 1,
      failed_operations: 0,
      in_progress_operations: 2,
      issues: [],
      provider_auth_missing_slices: 0,
      provider_auth_unconfigured_slices: 0,
      provider_auth_issues: [],
    },
  })
  const rendered = formatKernelHealth(unhealthy)

  assert.match(rendered, /slice operations settling: starting=1 stopping=1 in_progress=2/)
  assert.match(rendered, /next: wait for the slice operation to finish; run \/slice list to identify any stuck slice, then run \/slice doctor and inspect logs if it does not settle/)
  assert.doesNotMatch(rendered, /<slice>/)
})


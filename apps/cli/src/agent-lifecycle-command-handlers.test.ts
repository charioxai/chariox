import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./cli-types.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
  handleAgentFocusCommand,
} from "./agent-lifecycle-command-handlers.js"

test("agent list summary renders aliases and pluralization", () => {
  assert.equal(formatAgentListSummary([]), "no agents in session")
  const remoteAgent = agent({
    id: "agent-2",
    agent_ref: "agent-b",
    state: "Working",
    provider: "codex",
    model: "codex/gpt-5.4",
    worktree_id: "/repo/feature",
    remote_execution: {
      worker_kernel_id: "kernel-worker",
      worker_machine_id: "machine-worker",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-worker",
    },
    remote_extension_manifest_sync: {
      state: "stale",
      manifest_hash: "abcdef123456",
      pending_revoke: true,
      last_error: "worker offline",
    },
    extension_grants: [
      { kind: "mcp", name: "filesystem" },
      { kind: "script", name: "deploy", environment: "prod" },
    ],
  })
  assert.equal(
    formatAgentListSummary([
      agent({ agent_ref: "agent-a", alias: "builder" }),
      remoteAgent,
    ]),
    "2 agents: agent-a (builder) [Idle; opencode gpt-5.4; worktree worktree-1; local; 0 grants], agent-b [Working; codex/gpt-5.4; worktree /repo/feature; remote kernel-worker@machine-worker run run-worker; 2 grants; manifest stale abcdef12 pending revoke error worker offline]",
  )
  assert.match(
    formatAgentListSummary([remoteAgent], [slice({
      id: "slice-1",
      name: "devbox",
      status: "running",
      worker_kernel_id: "kernel-worker",
      worker_machine_id: "machine-worker",
      agent_ids: ["agent-2"],
    })]),
    /agent-b \[Working; codex\/gpt-5.4; worktree \/repo\/feature; slice devbox run run-worker;/,
  )
})

test("agent inspect summary renders placement, grants, manifest, and substitutes", () => {
  const summary = formatAgentInspectSummary(agent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "slice qa",
    state: "Working",
    provider: "codex",
    model: "codex/gpt-5.4",
    effort: "high",
    execution_mode_override: "plan",
    permission_level_override: "required",
    worktree_id: "/repo/feature",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-1",
    },
    extension_grants: [
      { kind: "mcp", name: "filesystem" },
      { kind: "skill", name: "review" },
    ],
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "abcdef1234567890",
      pending_revoke: true,
      last_error: "worker offline",
    },
    substitutes: [{ provider: "opencode", model: "zen", variant: "fast" }],
    active_substitute_index: 0,
  }), [slice({
    id: "slice-wrong",
    name: "wrong-by-worker",
    status: "running",
    worktree_id: "/repo/other",
    worker_kernel_id: "slice-kernel",
    worker_machine_id: "slice-machine",
    agent_ids: ["agent-other"],
  }), slice({
    id: "slice-1",
    name: "devbox",
    status: "running",
    worktree_id: "/repo/feature",
    worker_kernel_id: "slice-kernel",
    worker_machine_id: "slice-machine",
    agent_ids: ["agent-remote", "agent-helper"],
    provider_auth: [{
      provider: "codex",
      state: "authenticated",
      email: "dev@example.com",
      alias: "daily",
    }],
  })], {
    activeProviderRunId: "run-session",
    activeProviderRunAgentId: "agent-remote",
  }, {
    homeKernelId: "home-kernel",
    homeMachineId: "home-machine",
    ownerUserId: "user-1",
  })

  assert.match(summary, /agent-remote \(slice qa\) \[Working\]/)
  assert.match(summary, /home kernel: home-kernel@home-machine/)
  assert.match(summary, /session owner: user-1/)
  assert.match(summary, /placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=run-1\)/)
  assert.match(summary, /provider run: session=run-session, worker=run-1/)
  assert.match(summary, /slice: devbox \(id=slice-1, status=running, display=headless, worktree=\/repo\/feature, agents=2\)/)
  assert.match(summary, /slice provider accounts: codex=daily \(dev@example.com\)/)
  assert.match(summary, /extensions: 2 grants \(active tools home-proxy; skills snapshot; mcp=1, skill=1\)/)
  assert.match(summary, /remote extension sync: failed, pending revoke, hash=abcdef123456, error=worker offline/)
  assert.match(summary, /next=run \/extension sync-status agent-remote; run \/machine kernels slice-machine; use \/extension sync-retry agent-remote after worker connectivity is healthy/)
  assert.match(summary, /substitutes: \*0:opencode\/zen\/fast/)
})

test("agent summaries expose session and worker provider run pointers", () => {
  const remoteAgent = agent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    remote_execution: {
      worker_kernel_id: "kernel-worker",
      worker_machine_id: "machine-worker",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-worker",
    },
  })

  assert.match(
    formatAgentListSummary([remoteAgent], [], {
      activeProviderRunId: "run-session",
      activeProviderRunAgentId: "agent-remote",
    }),
    /session run run-session/,
  )
  assert.match(
    formatAgentInspectSummary(remoteAgent, [], {
      activeProviderRunId: "run-session",
      activeProviderRunAgentId: "agent-remote",
    }),
    /provider run: session=run-session, worker=run-worker/,
  )
  assert.match(
    formatAgentInspectSummary(remoteAgent, [], {
      activeProviderRunId: "run-session",
      activeProviderRunAgentId: null,
    }),
    /provider run: session=run-session owner unknown, worker=run-worker/,
  )
  assert.doesNotMatch(
    formatAgentListSummary([remoteAgent], [], {
      activeProviderRunId: "run-session",
      activeProviderRunAgentId: null,
    }),
    /session run run-session/,
  )
})

test("agent focus command applies focus, launches a run, and reports the focused agent", async () => {
  const agentA = agent({ id: "agent-a", agent_ref: "agent-a" })
  const agentB = agent({ id: "agent-b", agent_ref: "agent-b", provider: "codex", model: "codex/gpt-5.4" })
  const previousSession = session({ focused_agent_id: agentA.id, agents: [agentA, agentB] })
  const focusedSession = session({ focused_agent_id: agentB.id, agents: [agentA, agentB] })
  let flashedMessage = ""
  let launchedAgentId: string | null = null
  let appliedSessionId: string | null = null

  await handleAgentFocusCommand({
    isAttached: () => true,
    sessionState: () => previousSession,
    currentModelId: () => "opencode/gpt-5.4",
    currentVariantId: () => "high",
    providerRunState: () => null,
    multiAgentResponseLayout: () => "individual",
    maxAgentsPerScreen: () => 4,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    formatError: (error) => String(error),
    applySessionState: (nextSession) => { appliedSessionId = nextSession.focused_agent_id },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    cycleAgentFocus: async () => ({ agent: agentB, session: focusedSession }),
    launchAgentProviderRun: async (_provider, _model, _variant, agentId) => {
      launchedAgentId = agentId
      return providerRun({ agent_instance_id: agentId })
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => focusedSession,
    destroyAgent: async () => focusedSession,
    focusAgent: async () => ({ agent: agentB, session: focusedSession }),
    resolveSessionAgent: () => ({ agent: agentB, error: null }),
    formatAgentLabel: (entry) => entry?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
  }, ["focus", agentB.id])

  assert.equal(launchedAgentId, agentB.id)
  assert.equal(appliedSessionId, agentB.id)
  assert.equal(flashedMessage, "focused on agent agent-b")
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.4",
    effort: "medium",
    worktree_id: "worktree-1",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: ["attachment-1"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [agent()],
    workflows: [],
    workflow_runs: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "slice-1",
    owner_kernel_id: "kernel-home",
    owner_machine_id: "machine-home",
    backend: "local_docker",
    os: "linux",
    status: "running",
    worker_kernel_ref: "slice:slice-1",
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function providerRun(overrides: Partial<RuntimeProviderRun> = {}): RuntimeProviderRun {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "codex/gpt-5.4",
    variant: "high",
    usage_tokens_total: null,
    state: "Running",
    started_at_ms: 0,
    ...overrides,
  }
}

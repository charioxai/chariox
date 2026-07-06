import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./kernel-types.js"
import {
  isCompleteSessionSnapshot,
  resolveAttachTimeProviderLaunch,
  resolveLaunchTargetAgent,
  resolvePromptRecoveryProviderLaunch,
  resolveSessionAgentDefaults,
  resolveStoredAgentLaunch,
  sessionListEntryFromSession,
  upsertSessionListEntry,
} from "./session-lifecycle-state.js"

function makeAgent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "codex/gpt-5",
    effort: "medium",
    worktree_id: "/tmp/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-1"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 6,
    agents: [makeAgent("agent-a")],
    config_state: { version: 1, values: {} },
    workflows: [],
    workflow_runs: [],
    workflow_schedules: [],
    workflow_consoles: [],
    ...overrides,
  }
}

test("upsertSessionListEntry prepends new sessions and patches existing rows", () => {
  const current = [
    { id: "session-1", alias: "old", worktree_id: "/tmp/a", status: "Created" },
    {
      id: "session-2",
      alias: "stale",
      workspace_label: "Cached workspace",
      worktree_id: "/tmp/b",
      status: "Created",
    },
  ]

  assert.deepEqual(
    upsertSessionListEntry(current, { id: "session-3", alias: "new", worktree_id: "/tmp/c", status: "Active" }),
    [
      { id: "session-3", alias: "new", worktree_id: "/tmp/c", status: "Active" },
      ...current,
    ],
  )

  assert.deepEqual(
    upsertSessionListEntry(current, { id: "session-2", alias: "updated", worktree_id: "/tmp/b", status: "Active" }),
    [
      current[0],
      {
        id: "session-2",
        alias: "updated",
        workspace_label: "Cached workspace",
        worktree_id: "/tmp/b",
        status: "Active",
      },
    ],
  )
})

test("sessionListEntryFromSession preserves runtime row metadata when present", () => {
  const entry = sessionListEntryFromSession({
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    workspace_label: "Workspace",
    directory: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    worktree_label: "main",
    workspace_live_sync_mode: "managed",
    host_machine_id: "machine-1",
    host_daemon_id: "kernel-1",
    status: "Active",
    created_at_ms: 10,
    last_used_at_ms: 20,
    last_activity_at_ms: 30,
    last_prompt_sent_at_ms: 40,
    attachment_ids: ["att-1"],
  })

  assert.deepEqual(entry, {
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    workspace_label: "Workspace",
    directory: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    worktree_label: "main",
    workspace_live_sync_mode: "managed",
    host_machine_id: "machine-1",
    host_daemon_id: "kernel-1",
    kernel_id: "kernel-1",
    status: "Active",
    created_at_ms: 10,
    last_used_at_ms: 20,
    last_activity_at_ms: 30,
    last_prompt_sent_at_ms: 40,
    attachment_ids: ["att-1"],
  })
})

test("isCompleteSessionSnapshot requires full hydrated runtime fields", () => {
  const watchdogSession = makeSession()
  delete watchdogSession.workflow_schedules
  watchdogSession.workflow_watchdogs = []
  const incompleteSession = makeSession()
  delete (incompleteSession as Partial<RuntimeSession>).queued_prompts

  assert.equal(isCompleteSessionSnapshot({ id: "session-1" }), false)
  assert.equal(isCompleteSessionSnapshot(makeSession()), true)
  assert.equal(isCompleteSessionSnapshot(watchdogSession), true)
  assert.equal(isCompleteSessionSnapshot(incompleteSession), false)
})

test("resolveLaunchTargetAgent respects valid focus and rejects stale focus", () => {
  const session = makeSession({
    focused_agent_id: "agent-b",
    agents: [
      makeAgent("agent-a"),
      makeAgent("agent-b", { provider: "opencode" }),
    ],
  })

  assert.equal(resolveLaunchTargetAgent(session)?.id, "agent-b")
  assert.equal(resolveLaunchTargetAgent({ ...session, focused_agent_id: "missing-agent" }), null)
  assert.equal(resolveLaunchTargetAgent({ ...session, focused_agent_id: null })?.id, "agent-a")
})

test("resolveStoredAgentLaunch uses focused agent profile for existing sessions", () => {
  const session = makeSession({
    focused_agent_id: "agent-b",
    agents: [
      makeAgent("agent-a"),
      makeAgent("agent-b", {
        provider: "claude",
        model: "claude/sonnet-4.6",
        effort: "high",
      }),
    ],
    agent_defaults: {
      provider: "opencode",
      model: "kimi/k2.6",
      effort: "medium",
    },
  })

  assert.deepEqual(
    resolveStoredAgentLaunch(session, { provider: "codex", model: "codex/gpt-5", effort: "low" }, false),
    { provider: "claude", model: "claude/sonnet-4.6", effort: "high" },
  )
})

test("resolveStoredAgentLaunch falls back to session defaults for new or unfocused sessions", () => {
  const fallback = { provider: "codex", model: "codex/gpt-5", effort: "low" }
  const session = makeSession({
    focused_agent_id: "agent-a",
    agents: [makeAgent("agent-a", { provider: "default", model: "", effort: "" })],
    agent_defaults: {
      provider: "opencode",
      model: "kimi/k2.6",
      effort: "medium",
    },
  })

  assert.deepEqual(resolveStoredAgentLaunch(session, fallback, false), {
    provider: "opencode",
    model: "kimi/k2.6",
    effort: "medium",
  })
  assert.deepEqual(resolveStoredAgentLaunch(session, fallback, true), {
    provider: "opencode",
    model: "kimi/k2.6",
    effort: "medium",
  })
})

test("resolveAttachTimeProviderLaunch loads existing provider runs before launch decisions", () => {
  assert.deepEqual(
    resolveAttachTimeProviderLaunch(
      makeSession({ active_provider_run_id: " run-1 " }),
      { provider: "codex", model: "codex/gpt-5", effort: "low" },
      false,
    ),
    { action: "load_provider_run", providerRunId: "run-1" },
  )
})

test("resolveAttachTimeProviderLaunch launches with resolved focused agent defaults", () => {
  const session = makeSession({
    focused_agent_id: "agent-b",
    agents: [
      makeAgent("agent-a"),
      makeAgent("agent-b", {
        provider: "claude",
        model: "claude/sonnet-4.6",
        effort: "high",
      }),
    ],
  })

  assert.deepEqual(
    resolveAttachTimeProviderLaunch(
      session,
      { provider: "codex", model: "codex/gpt-5", effort: "low" },
      false,
    ),
    {
      action: "launch_provider_run",
      launch: { provider: "claude", model: "claude/sonnet-4.6", effort: "high" },
      targetAgent: session.agents[1],
      targetAgentId: "agent-b",
    },
  )
})

test("resolveAttachTimeProviderLaunch skips attach-time launches that cannot be local", () => {
  const fallback = { provider: "codex", model: "codex/gpt-5", effort: "low" }
  const remoteAgent = makeAgent("agent-a", {
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "worker-agent-1",
    },
  })

  assert.deepEqual(
    resolveAttachTimeProviderLaunch(makeSession({ agents: [] }), fallback, false),
    { action: "skip_launch", reason: "no_visible_agents", launch: fallback, targetAgent: null },
  )
  assert.deepEqual(
    resolveAttachTimeProviderLaunch(makeSession({ focused_agent_id: "missing" }), fallback, false),
    { action: "skip_launch", reason: "missing_focused_agent", launch: fallback, targetAgent: null },
  )
  assert.deepEqual(
    resolveAttachTimeProviderLaunch(makeSession({ agents: [remoteAgent] }), fallback, false),
    {
      action: "skip_launch",
      reason: "remote_backed_agent",
      launch: { provider: "codex", model: "codex/gpt-5", effort: "medium" },
      targetAgent: remoteAgent,
    },
  )
  assert.equal(
    resolveAttachTimeProviderLaunch(makeSession({ agents: [] }), fallback, true).action,
    "launch_provider_run",
  )
})

test("resolvePromptRecoveryProviderLaunch uses the explicit prompt target agent", () => {
  const session = makeSession({
    focused_agent_id: "agent-a",
    agents: [
      makeAgent("agent-a", {
        provider: "codex",
        model: "codex/gpt-5",
        effort: "medium",
      }),
      makeAgent("agent-b", {
        provider: "claude",
        model: "claude/sonnet-4.6",
        effort: "high",
      }),
    ],
    active_provider_run_id: "stale-run",
  })

  assert.deepEqual(
    resolvePromptRecoveryProviderLaunch(
      session,
      { provider: "opencode", model: "kimi/k2.6", effort: "low" },
      "agent-b",
    ),
    {
      action: "launch_provider_run",
      launch: { provider: "claude", model: "claude/sonnet-4.6", effort: "high" },
      targetAgent: session.agents[1],
      targetAgentId: "agent-b",
    },
  )
})

test("resolvePromptRecoveryProviderLaunch skips local recovery for non-local targets", () => {
  const fallback = { provider: "codex", model: "codex/gpt-5", effort: "low" }
  const remoteAgent = makeAgent("agent-a", {
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "worker-agent-1",
    },
  })

  assert.deepEqual(
    resolvePromptRecoveryProviderLaunch(makeSession({ agents: [] }), fallback, null),
    { action: "skip_launch", reason: "no_visible_agents", launch: fallback, targetAgent: null },
  )
  assert.deepEqual(
    resolvePromptRecoveryProviderLaunch(makeSession({ focused_agent_id: "agent-a" }), fallback, "missing"),
    {
      action: "skip_launch",
      reason: "missing_target_agent",
      launch: { provider: "codex", model: "codex/gpt-5", effort: "low" },
      targetAgent: null,
    },
  )
  assert.deepEqual(
    resolvePromptRecoveryProviderLaunch(makeSession({ agents: [remoteAgent] }), fallback, "agent-a"),
    {
      action: "skip_launch",
      reason: "remote_backed_agent",
      launch: { provider: "codex", model: "codex/gpt-5", effort: "medium" },
      targetAgent: remoteAgent,
    },
  )
})

test("resolveSessionAgentDefaults ignores blank or default provider settings", () => {
  assert.deepEqual(
    resolveSessionAgentDefaults({
      id: "session-1",
      agent_defaults: {
        provider: "default",
        model: " ",
        effort: null,
      },
    }, { provider: "codex", model: "codex/gpt-5", effort: "low" }),
    { provider: "codex", model: "codex/gpt-5", effort: "low" },
  )
})

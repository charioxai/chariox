import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  ProviderAccountProfile,
  RuntimeSession,
} from "./cli-types.js"
import {
  formatAgentSubstituteSummary,
  handleAgentSubstituteCommand,
} from "./agent-substitute-command-handlers.js"

test("agent substitute summary lists the fallback order and timeout", () => {
  assert.equal(formatAgentSubstituteSummary(agent({
    substitution_timeout_ms: 1500,
    substitutes: [
      { provider: "codex", model: "gpt-5.4", variant: "high" },
      { provider: "claude", model: "sonnet" },
    ],
  })), "agent-1 substitutes (2, timeout 1500ms):\n- 0: codex/gpt-5.4/high\n- 1: claude/sonnet")
})

test("agent substitute add parses profile flags and applies update", async () => {
  const currentAgent = agent()
  const currentSession = session({ agents: [currentAgent] })
  let appliedAction: Record<string, unknown> | null = null
  let flashedMessage = ""

  await handleAgentSubstituteCommand({
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    flashFooter: (message) => { flashedMessage = message },
    updateAgentSubstitutes: async (_sessionId, _agentId, action) => {
      appliedAction = action
      return { agent: currentAgent, session: currentSession }
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry) => entry?.agent_ref ?? "",
  }, ["substitute", "add", "codex", "gpt-5.4", "--variant", "high", "--kernel", "kernel-1"])

  assert.deepEqual(appliedAction, {
    Add: {
      provider: "codex",
      model: "gpt-5.4",
      variant: "high",
      account_profile: null,
      kernel_id: "kernel-1",
      worktree_id: null,
    },
  })
  assert.equal(flashedMessage, "agent-1 substitute added: codex/gpt-5.4/high")
})

test("agent substitute move reorders the fallback chain", async () => {
  const currentAgent = agent({
    provider: "claude",
    model: "claude-opus-4-8",
    substitutes: [
      { provider: "opencode", model: "opencode-go/deepseek-v4-pro" },
      { provider: "codex", model: "gpt-5.6-sol", variant: "high" },
    ],
  })
  const currentSession = session({ agents: [currentAgent] })
  const actions: Record<string, unknown>[] = []
  const makeDeps = () => ({
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    flashFooter: () => {},
    updateAgentSubstitutes: async (_sessionId: string, _agentId: string, action: Record<string, unknown>) => {
      actions.push(action)
      return { agent: currentAgent, session: currentSession }
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry: AgentInstance | null | undefined) => entry?.agent_ref ?? "",
  })

  await handleAgentSubstituteCommand(makeDeps(), ["substitute", "move", "1", "0"])

  assert.deepEqual(actions, [
    { Move: { from_index: 1, to_index: 0 } },
  ])
})

test("agent substitute add resolves an account alias to the stable profile id", async () => {
  const currentAgent = agent()
  const currentSession = session({ agents: [currentAgent] })
  let appliedAction: Record<string, unknown> | null = null
  let flashedMessage = ""

  await handleAgentSubstituteCommand({
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    flashFooter: (message) => { flashedMessage = message },
    updateAgentSubstitutes: async (_sessionId, _agentId, action) => {
      appliedAction = action
      return { agent: currentAgent, session: currentSession }
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry) => entry?.agent_ref ?? "",
    listProviderAccountProfiles: async () => [
      providerAccount({ profile_id: "codex-work-internal", label: "Work" }),
      providerAccount({ profile_id: "codex-personal", label: "personal", is_default: true }),
    ],
  }, ["substitute", "add", "codex", "gpt-5.4", "--account", "WORK"])

  assert.deepEqual(appliedAction, {
    Add: {
      provider: "codex",
      model: "gpt-5.4",
      variant: null,
      account_profile: "codex-work-internal",
      kernel_id: null,
      worktree_id: null,
    },
  })
  assert.equal(flashedMessage, "agent-1 substitute added: codex/gpt-5.4 · account Work")
})

test("agent substitute add rejects unknown account aliases without updating", async () => {
  const currentAgent = agent()
  const currentSession = session({ agents: [currentAgent] })
  let updateCalls = 0
  let flashedMessage = ""

  await handleAgentSubstituteCommand({
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    flashFooter: (message) => { flashedMessage = message },
    updateAgentSubstitutes: async () => {
      updateCalls += 1
      return { agent: currentAgent, session: currentSession }
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry) => entry?.agent_ref ?? "",
    listProviderAccountProfiles: async () => [
      providerAccount({ profile_id: "codex-work-internal", label: "Work" }),
    ],
  }, ["substitute", "add", "codex", "gpt-5.4", "--account", "missing"])

  assert.equal(updateCalls, 0)
  assert.equal(flashedMessage, "provider account alias missing was not found for codex")
})

test("agent substitute add rejects a dangling account flag without updating", async () => {
  const currentAgent = agent()
  const currentSession = session({ agents: [currentAgent] })
  let updateCalls = 0
  let flashedMessage = ""

  for (const args of [
    ["substitute", "add", "codex", "gpt-5.4", "--account"],
    ["substitute", "add", "codex", "gpt-5.4", "--account", "--kernel", "kernel-1"],
  ]) {
    await handleAgentSubstituteCommand({
      sessionState: () => currentSession,
      focusedAgentId: () => currentAgent.id,
      flashFooter: (message) => { flashedMessage = message },
      updateAgentSubstitutes: async () => {
        updateCalls += 1
        return { agent: currentAgent, session: currentSession }
      },
      applySessionState: () => {},
      refreshAgentPanes: async () => {},
      resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
      formatAgentLabel: (entry) => entry?.agent_ref ?? "",
    }, args)

    assert.equal(
      flashedMessage,
      "usage: /agent substitute add <provider> <model> [--variant v] [--account alias] [--kernel k] [--worktree dir] [--agent a]",
    )
  }
  assert.equal(updateCalls, 0)
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.4",
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
    project_id: "project-default",
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

function providerAccount(
  overrides: Partial<ProviderAccountProfile> = {},
): ProviderAccountProfile {
  return {
    owner_user_id: "owner-1",
    provider: "codex",
    profile_id: "profile-1",
    label: "Work",
    origin: "chariox_created",
    is_default: false,
    auth_state: "authenticated",
    identity_summary: null,
    plan: null,
    last_validated_at_ms: null,
    usage: {
      availability: "available",
      source: "provider",
    },
    ...overrides,
  } as ProviderAccountProfile
}

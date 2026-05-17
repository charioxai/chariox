import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  formatAgentSubstituteSummary,
  handleAgentSubstituteCommand,
} from "./agent-substitute-command-handlers.js"

test("agent substitute summary marks active substitutes and timeout", () => {
  assert.equal(formatAgentSubstituteSummary(agent({
    active_substitute_index: 1,
    substitution_timeout_ms: 1500,
    substitutes: [
      { provider: "codex", model: "gpt-5.4", variant: "high" },
      { provider: "claude", model: "sonnet" },
    ],
  })), "agent-1 substitutes (2, timeout 1500ms):\n- 0: codex/gpt-5.4/high\n* 1: claude/sonnet")
})

test("agent substitute add parses profile flags and applies update", async () => {
  const currentAgent = agent()
  const currentSession = session({ agents: [currentAgent] })
  let appliedAction: Record<string, unknown> | null = null
  let flashedMessage = ""

  await handleAgentSubstituteCommand({
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    currentModelId: () => "gpt-5.4",
    currentVariantId: () => "high",
    flashFooter: (message) => { flashedMessage = message },
    updateAgentSubstitutes: async (_sessionId, _agentId, action) => {
      appliedAction = action
      return { agent: currentAgent, session: currentSession }
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    launchAgentProviderRun: async () => providerRun(),
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry) => entry?.agent_ref ?? "",
  }, ["substitute", "add", "codex", "gpt-5.4", "--variant", "high", "--kernel", "kernel-1"])

  assert.deepEqual(appliedAction, {
    Add: {
      provider: "codex",
      model: "gpt-5.4",
      variant: "high",
      kernel_id: "kernel-1",
      worktree_id: null,
    },
  })
  assert.equal(flashedMessage, "agent-1 substitute added: codex/gpt-5.4/high")
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

function providerRun(): RuntimeProviderRun {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "gpt-5.4",
    variant: "high",
    usage_tokens_total: null,
    state: "Running",
    started_at_ms: 0,
  }
}

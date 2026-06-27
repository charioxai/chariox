import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, PromptQueueItem, RuntimeSession } from "../cli-types.js"
import { promptForAgent } from "./claude-bridge.js"

test("native Claude bridge ignores stale prompts when projected activity is idle", () => {
  const stalePrompt = prompt("prompt-stale", "agent-1")
  const runtimeSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: stalePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
  })

  assert.equal(promptForAgent(runtimeSession, "agent-1"), null)
})

test("native Claude bridge only uses prompt matching projected active turn", () => {
  const activePrompt = prompt("prompt-active", "agent-1")
  const stalePrompt = prompt("prompt-stale", "agent-1")
  const runtimeSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: activePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(promptForAgent(runtimeSession, "agent-1")?.id, "prompt-active")
  const projectedActivity = {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
      active_turn: {
        prompt_id: "prompt-active",
        status: "running",
        phase: "streaming",
      },
    },
  } satisfies NonNullable<RuntimeSession["agent_activity"]>

  const mismatchedSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: stalePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: projectedActivity,
  })

  assert.equal(promptForAgent(mismatchedSession, "agent-1"), null)
})

test("native Claude bridge falls back to legacy prompt fields before projected activity exists", () => {
  const activePrompt = prompt("prompt-legacy", "agent-1")

  assert.equal(promptForAgent(session({ active_prompt: activePrompt }), "agent-1")?.id, "prompt-legacy")
})

test("native Claude bridge prefers explicit prompt state over stale top-level prompt", () => {
  const activePrompt = prompt("prompt-stale", "agent-1")

  assert.equal(promptForAgent(session({
    active_prompt: activePrompt,
    prompt_states: {},
  }), "agent-1"), null)
  assert.equal(promptForAgent(session({
    active_prompt: activePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  }), "agent-1"), null)
})

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "claude",
      model: "sonnet-4.6",
      effort: "medium",
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [agent("agent-1")],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function agent(id: string): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "claude",
    model: "sonnet-4.6",
    effort: "medium",
    worktree_id: "/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}

function prompt(id: string, targetAgentId: string): PromptQueueItem {
  return {
    id,
    source_attachment_id: "attachment-1",
    target_agent_id: targetAgentId,
    prompt: "test prompt",
    status: "running",
  }
}

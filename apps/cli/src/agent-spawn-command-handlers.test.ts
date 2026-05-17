import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import { handleAgentSpawnCommand } from "./agent-spawn-command-handlers.js"

test("agent spawn command count inherits session defaults and launches each agent", async () => {
  let currentSession = session()
  const spawnedAgentIds: string[] = []
  const launchAgentIds: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => String(error),
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async (_provider, _model, _variant, agentId) => {
      launchAgentIds.push(agentId)
      return providerRun({ agent_instance_id: agentId })
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async () => {
      const id = `agent-${spawnedAgentIds.length + 1}`
      const nextAgent = agent({ id, agent_ref: id, provider: "codex", model: "codex/gpt-5.4", effort: "high" })
      spawnedAgentIds.push(id)
      currentSession = session({ focused_agent_id: id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => {},
  }, ["2"])

  assert.deepEqual(spawnedAgentIds, ["agent-1", "agent-2"])
  assert.deepEqual(launchAgentIds, ["agent-1", "agent-2"])
  assert.equal(flashedMessage, "spawned 2 agents from session defaults")
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-0",
    agent_ref: "agent-0",
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
    focused_agent_id: "agent-0",
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

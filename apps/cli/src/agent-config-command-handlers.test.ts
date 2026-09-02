import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  ProviderAccountProfile,
  RuntimeSession,
} from "./cli-types.js"
import {
  handleAgentModeCommand,
  handleAgentProfileCommand,
} from "./agent-config-command-handlers.js"

test("agent mode command applies explicit agent execution mode", async () => {
  const currentAgent = agent()
  const updatedAgent = agent({ execution_mode_override: "plan" })
  const updatedSession = session({ agents: [updatedAgent] })
  let updateOptions: Record<string, unknown> | null = null
  let flashedMessage = ""

  await handleAgentModeCommand({
    ...deps(currentAgent),
    flashFooter: (message) => { flashedMessage = message },
    updateAgentConfig: async (_sessionId, _agentId, options) => {
      updateOptions = options
      return { agent: updatedAgent, session: updatedSession }
    },
  }, ["mode", "agent-1", "plan"])

  assert.deepEqual(updateOptions, {
    executionMode: "plan",
    clearExecutionMode: false,
  })
  assert.equal(flashedMessage, "agent-1 mode: plan (agent)")
})

test("agent variant command clears agent effort override", async () => {
  const currentAgent = agent({ effort: "high" })
  const updatedAgent = agent({ effort: null })
  const updatedSession = session({ agents: [updatedAgent] })
  let updateOptions: Record<string, unknown> | null = null
  let flashedMessage = ""

  await handleAgentProfileCommand({
    ...deps(currentAgent),
    flashFooter: (message) => { flashedMessage = message },
    updateAgentProfile: async (_sessionId, _agentId, options) => {
      updateOptions = options
      return { agent: updatedAgent, session: updatedSession }
    },
  }, ["variant", "agent-1", "clear"], "variant")

  assert.deepEqual(updateOptions, { clearEffort: true })
  assert.equal(flashedMessage, "agent-1 variant: <none>")
})

test("agent provider command updates the targeted agent profile", async () => {
  const currentAgent = agent()
  const updatedAgent = agent({ provider: "claude-headless" })
  const updatedSession = session({ agents: [updatedAgent] })
  let updateOptions: Record<string, unknown> | null = null
  let appliedSession: RuntimeSession | null = null

  await handleAgentProfileCommand({
    ...deps(currentAgent),
    applySessionState: (nextSession) => { appliedSession = nextSession },
    updateAgentProfile: async (_sessionId, _agentId, options) => {
      updateOptions = options
      return { agent: updatedAgent, session: updatedSession }
    },
  }, ["provider", "agent-1", "claude-headless"], "provider")

  assert.deepEqual(updateOptions, { provider: "claude-headless" })
  assert.equal(appliedSession, updatedSession)
})

test("agent model command updates the targeted agent profile", async () => {
  const currentAgent = agent()
  const updatedAgent = agent({ model: "gpt-5.4" })
  let updateOptions: Record<string, unknown> | null = null
  let panesRefreshed = false

  await handleAgentProfileCommand({
    ...deps(currentAgent),
    refreshAgentPanes: async () => { panesRefreshed = true },
    updateAgentProfile: async (_sessionId, _agentId, options) => {
      updateOptions = options
      return { agent: updatedAgent, session: session({ agents: [updatedAgent] }) }
    },
  }, ["model", "agent-1", "gpt-5.4"], "model")

  assert.deepEqual(updateOptions, { model: "gpt-5.4" })
  assert.equal(panesRefreshed, true)
})

test("agent account command loads its scoped catalog and applies a compatible profile atomically", async () => {
  const currentAgent = agent({ provider: "codex", model: "gpt-5.4", effort: "high", account_profile: "default" })
  const updatedAgent = agent({
    provider: "codex",
    model: "codex/gpt-5.6-luna",
    effort: "low",
    account_profile: "secondary",
  })
  let updateOptions: Record<string, unknown> | null = null
  const catalogLoads: unknown[] = []
  let flashedMessage = ""

  const handlerDeps: Parameters<typeof handleAgentProfileCommand>[0] = {
    ...deps(currentAgent),
    flashFooter: (message) => { flashedMessage = message },
    listProviderAccountProfiles: async () => [{
      owner_user_id: "user-1",
      provider: "codex",
      profile_id: "secondary",
      label: "Validation",
      origin: "chariox_created",
      is_default: true,
      auth_state: "authenticated",
      identity_summary: "validation@example.com",
      usage: {
        profile_id: "secondary",
        provider: "codex",
        availability: "unavailable",
        source: "test",
      },
    } satisfies ProviderAccountProfile],
    getProviderCatalogForAgent: async (_agent, provider, accountProfile) => {
      catalogLoads.push({ provider, accountProfile })
      return {
        all: [{
          id: "codex",
          name: "Codex",
          models: {
            "gpt-5.6-luna": {
              id: "gpt-5.6-luna",
              name: "Luna",
              status: "active",
              variants: { low: {} },
            },
          },
        }],
        default: { codex: "gpt-5.6-luna" },
        connected: ["codex"],
      }
    },
    updateAgentProfile: async (_sessionId, _agentId, options) => {
      updateOptions = options
      return { agent: updatedAgent, session: session({ agents: [updatedAgent] }) }
    },
  }

  await handleAgentProfileCommand(handlerDeps, ["account", "agent-1", "Validation"], "account")
  await handleAgentProfileCommand(handlerDeps, ["account", "agent-1", "default"], "account")

  assert.deepEqual(catalogLoads, [
    { provider: "codex", accountProfile: "secondary" },
    { provider: "codex", accountProfile: "secondary" },
  ])
  assert.deepEqual(updateOptions, {
    provider: "codex",
    accountProfile: "secondary",
    model: "codex/gpt-5.6-luna",
    effort: "low",
  })
  assert.equal(flashedMessage, "agent-1 account: Validation")
})

test("agent account display resolves the virtual default pointer to its public alias", async () => {
  const currentAgent = agent({ provider: "codex", account_profile: "default" })
  let flashedMessage = ""

  await handleAgentProfileCommand({
    ...deps(currentAgent),
    flashFooter: (message) => { flashedMessage = message },
    updateAgentProfile: async () => ({ agent: currentAgent, session: session({ agents: [currentAgent] }) }),
    listProviderAccountProfiles: async () => [{
      owner_user_id: "user-1",
      provider: "codex",
      profile_id: "opaque-profile-id",
      label: "codex-1",
      origin: "default",
      is_default: true,
      auth_state: "authenticated",
      usage: {
        profile_id: "opaque-profile-id",
        provider: "codex",
        availability: "unavailable",
        source: "test",
      },
    } satisfies ProviderAccountProfile],
  }, ["account"], "account")

  assert.equal(flashedMessage, "agent-1 account: codex-1")
})

function deps(currentAgent: AgentInstance) {
  const currentSession = session({ agents: [currentAgent] })
  return {
    sessionState: () => currentSession,
    focusedAgentId: () => currentAgent.id,
    flashFooter: () => {},
    formatError: (error: unknown) => String(error),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    resolveSessionAgent: () => ({ agent: currentAgent, error: null }),
    formatAgentLabel: (entry: AgentInstance | null | undefined) => entry?.agent_ref ?? "",
  }
}

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

import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeProviderRun, RuntimeSession } from "./kernel-types.js"
import {
  applyProviderRunProfileToSession,
  derivePromptProviderSelection,
  normalizePromptProvider,
  providerRunForPromptSelection,
  resolveProviderModelContextLimit,
  splitProviderModelRef,
} from "./prompt-provider-selection.js"

test("derivePromptProviderSelection prefers a focused provider run then agent and waiting room defaults", () => {
  assert.deepEqual(
    derivePromptProviderSelection({
      providerRun: providerRun({
        provider: "openai",
        model: "opencode/gpt-5.4",
        variant: "low",
      }),
      waitingRoomState: {
        modelId: "anthropic/claude-sonnet-4",
        effort: "medium",
      },
      defaultModel: "default",
      defaultEffort: "high",
    }),
    {
      provider: "openai",
      model: "opencode/gpt-5.4",
      effort: "low",
    },
  )

  assert.deepEqual(
    derivePromptProviderSelection({
      providerRun: null,
      focusedAgent: agent("agent-a", {
        provider: "codex",
        model: "openai/gpt-5.3-codex",
        effort: "medium",
      }),
      waitingRoomState: {
        providerId: "claude",
        modelId: "anthropic/claude-sonnet-4",
        effort: "high",
      },
      defaultModel: "default",
      defaultEffort: "normal",
    }),
    {
      provider: "codex",
      model: "openai/gpt-5.3-codex",
      effort: "medium",
    },
  )
})

test("derivePromptProviderSelection resolves to null fields when nothing is configured", () => {
  assert.deepEqual(
    derivePromptProviderSelection({
      providerRun: null,
      waitingRoomState: {},
    }),
    {
      provider: null,
      model: null,
      effort: null,
    },
  )
})

test("derivePromptProviderSelection uses the configured default provider/model/effort", () => {
  assert.deepEqual(
    derivePromptProviderSelection({
      providerRun: null,
      waitingRoomState: {},
      defaultProvider: "codex",
      defaultModel: "gpt-5.1",
      defaultEffort: "high",
    }),
    {
      provider: "codex",
      model: "gpt-5.1",
      effort: "high",
    },
  )
})

test("providerRunForPromptSelection requires focused-agent ownership when a focused agent exists", () => {
  const run = providerRun({ agent_instance_id: "agent-a" })

  assert.equal(providerRunForPromptSelection(run, null), run)
  assert.equal(providerRunForPromptSelection(run, agent("agent-a")), run)
  assert.equal(providerRunForPromptSelection(run, agent("agent-b")), null)
  assert.equal(providerRunForPromptSelection(providerRun({ agent_instance_id: null }), agent("agent-a")), null)
})

test("derivePromptProviderSelection does not infer focused agent ownership from provider run", () => {
  assert.deepEqual(
    derivePromptProviderSelection({
      providerRun: providerRun({
        agent_instance_id: null,
        provider: "openai",
        model: "opencode/gpt-5.4",
        variant: "low",
      }),
      focusedAgent: agent("agent-a", {
        provider: "codex",
        model: "openai/gpt-5.3-codex",
        effort: "medium",
      }),
      waitingRoomState: {
        providerId: "claude-p",
        modelId: "anthropic/claude-sonnet-4",
        effort: "high",
      },
      defaultModel: "default",
      defaultEffort: "normal",
    }),
    {
      provider: "codex",
      model: "openai/gpt-5.3-codex",
      effort: "medium",
    },
  )
})

test("applyProviderRunProfileToSession overlays accepted run profile onto the matching agent", () => {
  const projected = applyProviderRunProfileToSession(
    session({
      active_provider_run_id: "run-1",
      agents: [
        agent("agent-1", { provider: "opencode", model: "opencode/gpt-5.4", effort: "high" }),
        agent("agent-2", { provider: "codex", model: "gpt-5.4", effort: "medium" }),
      ],
    }),
    providerRun({
      agent_instance_id: "agent-1",
      provider: "opencode",
      model: "opencode/gpt-5.4",
      variant: "low",
    }),
  )

  assert.equal(projected.agents[0]?.effort, "low")
  assert.equal(projected.agents[0]?.model, "opencode/gpt-5.4")
  assert.equal(projected.agents[1]?.effort, "medium")
})

test("a replaced leased run cannot overwrite the kernel-selected return model", () => {
  const current = session({
    active_provider_run_id: "leased:lease-agent:provider-run-5",
    agents: [agent("agent-1", { provider: "claude-p", model: "sonnet", effort: "high" })],
  })
  const projected = applyProviderRunProfileToSession(current, providerRun({
    id: "leased:lease-agent:provider-run-4",
    agent_instance_id: "agent-1",
    provider: "claude-p",
    model: "claude-haiku-4-5-20251001",
    variant: "high",
  }))

  assert.equal(projected, current)
  assert.equal(projected.agents[0]?.model, "sonnet")
})

test("a cleared run cannot resurrect its old profile in a kernel snapshot", () => {
  const current = session({
    active_provider_run_id: null,
    agents: [agent("agent-1", { provider: "codex", model: "gpt-6.1-sol", effort: "high" })],
  })
  const projected = applyProviderRunProfileToSession(current, providerRun({
    agent_instance_id: "agent-1",
    provider: "claude-p",
    model: "claude-haiku-4-5-20251001",
    variant: "high",
  }))

  assert.equal(projected, current)
  assert.equal(projected.agents[0]?.provider, "codex")
})

test("provider context limit resolves provider/model references", () => {
  const catalog = {
    all: [{
      id: "opencode",
      models: {
        "gpt-5.4": {
          limit: {
            context: 100_000,
          },
        },
      },
    }],
  }

  assert.equal(resolveProviderModelContextLimit(catalog, "opencode", "gpt-5.4"), 100_000)
  assert.equal(resolveProviderModelContextLimit(catalog, "ignored", "opencode/gpt-5.4"), 100_000)
  assert.equal(resolveProviderModelContextLimit(catalog, "opencode", "missing"), null)
})

test("provider helpers normalize default and split model refs", () => {
  assert.equal(normalizePromptProvider("default"), null)
  assert.equal(normalizePromptProvider("codex"), "codex")
  assert.deepEqual(splitProviderModelRef("openai/gpt-5.4"), { providerId: "openai", modelId: "gpt-5.4" })
  assert.deepEqual(splitProviderModelRef("gateway/openai/gpt-5.4"), { providerId: "openai", modelId: "gpt-5.4" })
  assert.equal(splitProviderModelRef("gpt-5.4"), null)
})

function providerRun(overrides: Partial<RuntimeProviderRun> = {}): RuntimeProviderRun {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: null,
    adapter_key: "adapter",
    provider: "openai",
    account_profile: "default",
    model: "opencode/gpt-5.4",
    variant: "high",
    usage_tokens_total: null,
    state: "active",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/workspace/tree",
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

test("a lagging run of the previous provider cannot revert a committed provider switch", () => {
  const committed = session({
    active_provider_run_id: "run-claude",
    agents: [agent("agent-1", { provider: "codex", model: "gpt-5.6-luna", effort: "high" })],
  })
  const projected = applyProviderRunProfileToSession(committed, providerRun({
    id: "run-claude",
    agent_instance_id: "agent-1",
    provider: "claude-p",
    model: "claude-haiku-5-5",
    variant: "low",
  }))

  assert.equal(projected, committed)
})

test("the previous provider's run does not describe an agent switched away from it", () => {
  const run = providerRun({ id: "run-claude", agent_instance_id: "agent-1", provider: "claude-p", model: "claude-haiku-5-5" })

  assert.equal(providerRunForPromptSelection(run, agent("agent-1", { provider: "codex", model: "gpt-5.6-luna" })), null)
  assert.equal(providerRunForPromptSelection(run, agent("agent-1", { provider: "claude-p", model: "claude-haiku-5-5" })), run)
})

test("a current cross-provider turn substitute still describes its agent", () => {
  const substituteRun = providerRun({
    id: "run-substitute",
    agent_instance_id: "agent-1",
    provider: "claude-p",
    account_profile: "claude-backup",
    model: "claude-haiku-5-5",
    variant: "low",
  })
  const primary = agent("agent-1", {
    provider: "codex",
    model: "gpt-5.6-luna",
    effort: "high",
    substitutes: [{ provider: "claude-p", model: "claude-haiku-5-5", variant: "low", account_profile: "claude-backup" }],
  })

  assert.equal(providerRunForPromptSelection(substituteRun, primary), substituteRun)
  const projected = applyProviderRunProfileToSession(
    session({ active_provider_run_id: "run-substitute", agents: [primary] }),
    substituteRun,
  )
  assert.deepEqual(
    [projected.agents[0]?.provider, projected.agents[0]?.model, projected.agents[0]?.effort],
    ["claude-p", "claude-haiku-5-5", "low"],
  )
})

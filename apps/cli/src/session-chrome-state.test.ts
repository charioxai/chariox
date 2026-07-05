import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeProviderRun } from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  derivePromptMetaState,
  derivePromptUsageState,
} from "./session-chrome-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("derivePromptMetaState formats provider, model, and effort from the current selection", () => {
  const parts = derivePromptMetaState({
    providerRun: providerRun({
      provider: "opencode",
      model: "opencode/gpt-5.4",
      variant: "high",
    }),
    waitingRoomState: waitingRoomState(),
    defaultModel: "default",
    defaultEffort: "medium",
  })

  assert.deepEqual(
    parts.map((part) => part.text),
    ["OpenCode", "OpenCode GPT-5.4", "High"],
  )
})

test("derivePromptUsageState resolves usage metadata from the provider catalog", () => {
  const usage = derivePromptUsageState({
    providerRun: providerRun({
      agent_instance_id: "agent-1",
      provider: "openai",
      model: "opencode/gpt-5.4",
      usage_tokens_total: 42_100,
      usage: {
        context_tokens: 4096,
      },
    }),
    focusedAgent: agent("agent-1"),
    catalog: catalog(),
  })

  assert.equal(usage?.tokensLabel, "42,100 tok")
  assert.equal(usage?.usagePercent, 4)
  assert.equal(usage?.usageLabel, "4%")

  const unownedUsage = derivePromptUsageState({
    providerRun: providerRun({
      agent_instance_id: null,
      usage_tokens_total: 42_100,
    }),
    focusedAgent: agent("agent-1"),
    catalog: catalog(),
  })
  assert.equal(unownedUsage, null)

  const otherAgentUsage = derivePromptUsageState({
    providerRun: providerRun({
      agent_instance_id: "agent-2",
      usage_tokens_total: 42_100,
    }),
    focusedAgent: agent("agent-1"),
    catalog: catalog(),
  })
  assert.equal(otherAgentUsage, null)
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "existing:/workspace",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "high",
    themeId: "opencode",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

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

function catalog(): ProviderCatalog {
  return {
    all: [
      {
        id: "opencode",
        name: "OpenCode Zen",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            limit: {
              context: 100_000,
            },
            variants: {
              low: {},
              high: {},
            },
          },
        },
      },
    ],
    default: {
      opencode: "gpt-5.4",
    },
    connected: ["opencode"],
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { createProviderPromptProjectionController } from "./provider-prompt-projection-controller.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("provider prompt projection prefers provider run, then focused agent, then waiting room/defaults", () => {
  let run: RuntimeProviderRun | null = providerRun({
    agent_instance_id: "agent-a",
    provider: "openai",
    model: "opencode/gpt-5.4",
    variant: "low",
  })
  const controller = createProviderPromptProjectionController({
    getProviderRun: () => run,
    getFocusedAgent: () => agent("agent-a", {
      provider: "codex",
      model: "openai/gpt-5.3-codex",
      effort: "medium",
    }),
    getWaitingRoomState: () => waitingRoomState({
      providerId: "opencode",
      modelId: "anthropic/claude-sonnet-4",
      effort: "high",
    }),
    getDefaults: () => ({
      provider: "opencode",
      model: "default-model",
      effort: "default-effort",
    }),
    getProviderCatalog: catalog,
  })

  assert.deepEqual(controller.currentProviderSelection(), {
    provider: "openai",
    model: "opencode/gpt-5.4",
    effort: "low",
  })
  assert.equal(controller.currentModelId(), "opencode/gpt-5.4")
  assert.equal(controller.currentVariantId(), "low")

  run = providerRun({
    agent_instance_id: "agent-b",
    provider: "openai",
    model: "opencode/gpt-5.4",
    variant: "low",
  })

  assert.deepEqual(controller.currentProviderSelection(), {
    provider: "codex",
    model: "openai/gpt-5.3-codex",
    effort: "medium",
  })

  run = null

  assert.deepEqual(controller.currentProviderSelection(), {
    provider: "codex",
    model: "openai/gpt-5.3-codex",
    effort: "medium",
  })
})

test("provider prompt projection derives prompt meta and usage", () => {
  let focusedAgent: AgentInstance | null = null
  let run: RuntimeProviderRun | null = providerRun({
    provider: "opencode",
    model: "opencode/gpt-5.4",
    variant: "high",
    usage_tokens_total: 42_100,
    usage: {
      context_tokens: 4096,
    },
  })
  const controller = createProviderPromptProjectionController({
    getProviderRun: () => run,
    getFocusedAgent: () => focusedAgent,
    getWaitingRoomState: () => waitingRoomState(),
    getDefaults: () => ({
      provider: "opencode",
      model: "default-model",
      effort: "medium",
    }),
    getProviderCatalog: catalog,
  })

  assert.deepEqual(
    controller.promptMetaParts().map((part) => part.text),
    ["OpenCode", "OpenCode GPT-5.4", "High"],
  )
  assert.equal(controller.promptUsageMeta()?.tokensLabel, "42,100 tok")
  assert.equal(controller.promptUsageMeta()?.usageLabel, "4%")

  focusedAgent = agent("agent-a")
  run = providerRun({
    agent_instance_id: "agent-b",
    provider: "opencode",
    model: "opencode/gpt-5.4",
    variant: "high",
    usage_tokens_total: 42_100,
    usage: {
      context_tokens: 4096,
    },
  })

  assert.deepEqual(
    controller.promptMetaParts().map((part) => part.text),
    ["OpenCode", "Default", "High"],
  )
  assert.equal(controller.promptUsageMeta(), null)
})

test("provider prompt projection uses owned provider run metadata for focused agent", () => {
  const controller = createProviderPromptProjectionController({
    getProviderRun: () => providerRun({
      agent_instance_id: "agent-a",
      provider: "opencode",
      model: "opencode/gpt-5.4",
      variant: "high",
      usage_tokens_total: 42_100,
      usage: {
        context_tokens: 4096,
      },
    }),
    getFocusedAgent: () => agent("agent-a"),
    getWaitingRoomState: () => waitingRoomState(),
    getDefaults: () => ({
      provider: "opencode",
      model: "default-model",
      effort: "medium",
    }),
    getProviderCatalog: catalog,
  })

  assert.deepEqual(
    controller.promptMetaParts().map((part) => part.text),
    ["OpenCode", "OpenCode GPT-5.4", "High"],
  )
  assert.equal(controller.promptUsageMeta()?.tokensLabel, "42,100 tok")
  assert.equal(controller.promptUsageMeta()?.usageLabel, "4%")
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

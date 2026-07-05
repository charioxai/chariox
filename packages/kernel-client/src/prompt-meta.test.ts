import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeProviderRun } from "./kernel-types.js"
import type { ProviderModelContextCatalog } from "./prompt-provider-selection.js"
import {
  derivePromptMetaState,
  derivePromptUsageState,
  formatPromptMetaLine,
  formatPromptMetaParts,
  formatPromptUsageMeta,
} from "./prompt-meta.js"

test("formatPromptMetaLine renders provider, model, and effort values", () => {
  assert.equal(formatPromptMetaLine("opencode", "gpt-5.4", "high"), "OpenCode • GPT-5.4 • High")
})

test("formatPromptMetaLine handles defaults and provider-qualified models", () => {
  assert.equal(formatPromptMetaLine("opencode", "opencode/gpt-5.4", ""), "OpenCode • OpenCode GPT-5.4")
  assert.equal(formatPromptMetaLine("opencode", "github-copilot/gpt-5.4", ""), "OpenCode • GitHub-Copilot GPT-5.4")
  assert.equal(formatPromptMetaLine("opencode", "default", "default"), "OpenCode • Default")
})

test("formatPromptMetaParts assigns bright tones per value", () => {
  assert.deepEqual(formatPromptMetaParts("opencode", "opencode/gpt-5.4", "high"), [
    { kind: "provider", text: "OpenCode", tone: "primary" },
    { kind: "model", text: "OpenCode GPT-5.4", tone: "secondary" },
    { kind: "variant", text: "High", tone: "primary" },
  ])
  assert.deepEqual(formatPromptMetaParts("anthropic", "claude-3.7-sonnet", "low"), [
    { kind: "provider", text: "Anthropic", tone: "warning" },
    { kind: "model", text: "Claude-3.7-Sonnet", tone: "warning" },
    { kind: "variant", text: "Low", tone: "success" },
  ])
})

test("formatPromptUsageMeta renders token totals and usage bars", () => {
  assert.deepEqual(formatPromptUsageMeta(42100, 12345, 20000, 10), {
    tokensLabel: "42,100 tok",
    usagePercent: 62,
    usageLabel: "62%",
    barFilled: "======",
    barEmpty: "----",
  })
})

test("formatPromptUsageMeta falls back to token-only metadata without a limit", () => {
  assert.deepEqual(formatPromptUsageMeta(512, null, null, 8), {
    tokensLabel: "512 tok",
    usagePercent: null,
    usageLabel: "",
    barFilled: "",
    barEmpty: "--------",
  })
})

test("formatPromptUsageMeta ignores impossible context usage", () => {
  assert.deepEqual(formatPromptUsageMeta(36_000_000, 36_000_000, 128_000, 8), {
    tokensLabel: "36,000,000 tok",
    usagePercent: null,
    usageLabel: "",
    barFilled: "",
    barEmpty: "--------",
  })
})

test("derivePromptMetaState formats provider, model, and effort from the current selection", () => {
  const parts = derivePromptMetaState({
    providerRun: providerRun({
      provider: "opencode",
      model: "opencode/gpt-5.4",
      variant: "high",
    }),
    waitingRoomState: {
      providerId: "opencode",
      modelId: "opencode/gpt-5.4",
      effort: "high",
    },
    defaultModel: "default",
    defaultEffort: "medium",
  })

  assert.deepEqual(
    parts.map((part) => part.text),
    ["OpenCode", "OpenCode GPT-5.4", "High"],
  )
})

test("derivePromptUsageState resolves usage metadata from provider catalog", () => {
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

function catalog(): ProviderModelContextCatalog {
  return {
    all: [
      {
        id: "opencode",
        models: {
          "gpt-5.4": {
            limit: {
              context: 100_000,
            },
          },
        },
      },
    ],
  }
}

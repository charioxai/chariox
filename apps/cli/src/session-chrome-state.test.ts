import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  deriveAttachedFooterSummary,
  deriveCurrentProviderSelection,
  deriveFooterHint,
  deriveFocusedStatusBadge,
  derivePromptMetaState,
  derivePromptUsageState,
  deriveSessionStatusMode,
  deriveVisibleActivityLabel,
} from "./session-chrome-state.js"
import type { WaitingRoomState } from "./waiting-room.js"

test("deriveCurrentProviderSelection prefers provider run and falls back to waiting-room values", () => {
  assert.deepEqual(
    deriveCurrentProviderSelection({
      providerRun: providerRun({
        provider: "openai",
        model: "openai/gpt-5.4",
        variant: "low",
      }),
      waitingRoomState: waitingRoomState({
        modelId: "anthropic/claude-sonnet-4",
        effort: "medium",
      }),
      defaultModel: "default",
      defaultEffort: "high",
    }),
    {
      provider: "openai",
      model: "openai/gpt-5.4",
      effort: "low",
    },
  )

  assert.deepEqual(
    deriveCurrentProviderSelection({
      providerRun: null,
      waitingRoomState: waitingRoomState({
        modelId: "anthropic/claude-sonnet-4",
        effort: "medium",
      }),
      defaultModel: "default",
      defaultEffort: "high",
    }),
    {
      provider: "opencode",
      model: "anthropic/claude-sonnet-4",
      effort: "medium",
    },
  )
})

test("derivePromptMetaState formats provider, model, and effort from the current selection", () => {
  const parts = derivePromptMetaState({
    providerRun: providerRun({
      provider: "openai",
      model: "openai/gpt-5.4",
      variant: "high",
    }),
    waitingRoomState: waitingRoomState(),
    defaultModel: "default",
    defaultEffort: "medium",
  })

  assert.deepEqual(
    parts.map((part) => part.text),
    ["OpenAI", "OpenAI GPT-5.4", "High"],
  )
})

test("derivePromptUsageState resolves usage metadata from the provider catalog", () => {
  const usage = derivePromptUsageState({
    providerRun: providerRun({
      provider: "openai",
      model: "openai/gpt-5.4",
      usage_tokens_total: 4096,
    }),
    catalog: catalog(),
  })

  assert.equal(usage?.tokensLabel, "4,096 tok")
  assert.equal(usage?.usagePercent, 4)
  assert.equal(usage?.usageLabel, "4%")
})

test("deriveSessionStatusMode and footer hint reflect prompt and failure state", () => {
  assert.equal(
    deriveSessionStatusMode({
      daemonDisconnected: true,
      working: false,
      hasActivePrompt: false,
      submitting: false,
      queueDepth: 0,
    }),
    "disconnected",
  )
  assert.equal(
    deriveSessionStatusMode({
      daemonDisconnected: false,
      working: false,
      hasActivePrompt: true,
      submitting: false,
      queueDepth: 0,
    }),
    "working",
  )
  assert.equal(
    deriveFooterHint({
      fatalError: null,
      activePromptId: "prompt-1",
      queueDepth: 2,
      statusLine: "Connected.",
    }),
    "Processing prompt-1; 2 queued.",
  )
  assert.equal(
    deriveFooterHint({
      fatalError: "boom",
      activePromptId: "prompt-1",
      queueDepth: 0,
      statusLine: "Connected.",
    }),
    "boom",
  )
})

test("deriveAttachedFooterSummary includes focused agent, view mode, and hotkey hint", () => {
  const summary = deriveAttachedFooterSummary({
    session: session({
      alias: "feature-refactor",
      attachment_ids: ["cli-1", "cli-2"],
      focused_agent_id: "agent-b",
      agents: [
        agent("agent-a", { agent_ref: "main" }),
        agent("agent-b", {
          agent_ref: "review",
          alias: "QA",
          is_processing: true,
        }),
      ],
    }),
    connectedClientCount: 2,
    multiAgentMode: true,
    responseLayout: "split",
    sessionStatusMode: "working",
    hotkeyToggleLabel: "Ctrl+T",
  })

  assert.equal(
    summary,
    "Session feature-refactor • 2 CLIs connected • 2 agents in session • Agent: review (QA) [working] • View: split • Ctrl+C to stop • Tab cycles agents • Ctrl+Tab opens workflow • Ctrl+T hotkeys",
  )
})

test("deriveVisibleActivityLabel prefers active tool activity over provider activity", () => {
  assert.equal(
    deriveVisibleActivityLabel({
      providerActivityLabel: "thinking",
      activeToolLabels: ["reading", "patching"],
    }),
    "patching",
  )
  assert.equal(
    deriveVisibleActivityLabel({
      providerActivityLabel: "thinking",
      activeToolLabels: [],
    }),
    "thinking",
  )
})

test("deriveFocusedStatusBadge handles unattached, disconnected, and streaming focused agents", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: false,
      daemonDisconnected: false,
      focusedAgent: null,
      focusedAgentActivityLabel: null,
      streamingAgentId: null,
    }),
    { label: "", tone: "idle" },
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: true,
      focusedAgent: agent("agent-a", { state: "Working", is_processing: true }),
      focusedAgentActivityLabel: "reading",
      streamingAgentId: "agent-a",
    }),
    { label: "DISCONNECTED", tone: "disconnected" },
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      focusedAgent: agent("agent-a", { state: "Idle", is_processing: false }),
      focusedAgentActivityLabel: null,
      streamingAgentId: "agent-a",
    }),
    { label: "WORKING", tone: "working" },
  )
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    modelId: "openai/gpt-5.4",
    effort: "high",
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
    model: "openai/gpt-5.4",
    variant: "high",
    usage_tokens_total: null,
    state: "active",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
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

function catalog(): ProviderCatalog {
  return {
    all: [
      {
        id: "openai",
        name: "OpenAI",
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
      openai: "gpt-5.4",
    },
    connected: ["openai"],
  }
}

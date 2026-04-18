import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  applyProviderRunProfileToSession,
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

test("applyProviderRunProfileToSession overlays accepted run profile onto the matching agent", () => {
  const projected = applyProviderRunProfileToSession(
    session({
      agents: [
        agent("agent-1", { provider: "opencode", model: "openai/gpt-5.4", effort: "high" }),
        agent("agent-2", { provider: "codex", model: "gpt-5.4", effort: "medium" }),
      ],
    }),
    providerRun({
      agent_instance_id: "agent-1",
      provider: "opencode",
      model: "openai/gpt-5.4",
      variant: "low",
    }),
  )

  assert.equal(projected.agents[0]?.effort, "low")
  assert.equal(projected.agents[0]?.model, "openai/gpt-5.4")
  assert.equal(projected.agents[1]?.effort, "medium")
})

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
      focusedAgent: agent("agent-a", {
        provider: "codex",
        model: "openai/gpt-5.3-codex",
        effort: "medium",
      }),
      waitingRoomState: waitingRoomState({
        modelId: "anthropic/claude-sonnet-4",
        effort: "medium",
      }),
      defaultModel: "default",
      defaultEffort: "high",
    }),
    {
      provider: "codex",
      model: "openai/gpt-5.3-codex",
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

test("deriveAttachedFooterSummary includes view mode and hotkey hint without focused agent details", () => {
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
    "Session feature-refactor • 2 CLIs connected • 2 agents in session • Ctrl+C to stop • Tab cycles focus • Ctrl+P opens workflow • Ctrl+T hotkeys",
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

test("deriveFocusedStatusBadge follows session-level working state", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: false,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: false,
    }),
    badge([]),
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: true,
      activeStatusLabel: "reading",
      focusedBusy: true,
    }),
    badge([{ label: "DISCONNECTED", tone: "disconnected" }]),
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: true,
    }),
    badge([{ label: "THINKING", tone: "working" }]),
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: true,
    }),
    badge([{ label: "THINKING", tone: "working" }]),
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: false,
    }),
    badge([{ label: "IDLE", tone: "idle" }]),
  )

  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: "reading",
      focusedBusy: true,
    }),
    badge([{ label: "READING", tone: "working" }]),
  )
})

test("deriveFocusedStatusBadge stays working while the focused agent is busy", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: true,
    }),
    badge([{ label: "THINKING", tone: "working" }]),
  )
})

test("deriveFocusedStatusBadge shows single agent status without agents array", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: false,
    }),
    badge([{ label: "IDLE", tone: "idle" }]),
  )
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: "reading",
      focusedBusy: true,
    }),
    badge([{ label: "READING", tone: "working" }]),
  )
})

test("deriveFocusedStatusBadge shows N IDLE when all agents are idle", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: false,
      agents: [
        { id: "agent-1", busy: false },
        { id: "agent-2", busy: false },
        { id: "agent-3", busy: false },
      ],
    }),
    badge([{ label: "3 IDLE", tone: "idle" }]),
  )
})

test("deriveFocusedStatusBadge shows N WORKING when all agents are working", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: true,
      agents: [
        { id: "agent-1", busy: true },
        { id: "agent-2", busy: true },
      ],
    }),
    badge([{ label: "2 WORKING", tone: "working" }]),
  )
})

test("deriveFocusedStatusBadge shows X IDLE Y WORKING for mixed states", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: "reading",
      focusedBusy: true,
      agents: [
        { id: "agent-1", busy: false },
        { id: "agent-2", busy: true },
        { id: "agent-3", busy: false },
        { id: "agent-4", busy: true },
      ],
    }),
    badge([
      { label: "2 IDLE", tone: "idle" },
      { label: "2 WORKING", tone: "working" },
    ]),
  )
})

test("deriveFocusedStatusBadge shows single agent IDLE/WORKING with one agent in array", () => {
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: null,
      focusedBusy: false,
      agents: [{ id: "agent-1", busy: false }],
    }),
    badge([{ label: "IDLE", tone: "idle" }]),
  )
  assert.deepEqual(
    deriveFocusedStatusBadge({
      attached: true,
      daemonDisconnected: false,
      activeStatusLabel: "patching",
      focusedBusy: true,
      agents: [{ id: "agent-1", busy: true }],
    }),
    badge([{ label: "PATCHING", tone: "working" }]),
  )
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    providerId: "opencode",
    modelId: "openai/gpt-5.4",
    effort: "high",
    themeId: "opencode",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

function badge(parts: Array<{ label: string; tone: "idle" | "working" | "disconnected" | "error" }>) {
  return {
    label: parts.map((part) => part.label).join(" "),
    tone: parts.some((part) => part.tone === "working")
      ? "working"
      : parts[0]?.tone ?? "idle",
    parts,
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

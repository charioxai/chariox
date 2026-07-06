import assert from "node:assert/strict"
import test from "node:test"

import { createAgentRuntimeProjectionController } from "./agent-runtime-projection-controller.js"
import type {
  AgentInstance,
  PromptQueueItem,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"

test("agent runtime projection resolves focused agent, prompt work, and tool activity", () => {
  const session = runtimeSession({
    focused_agent_id: "agent-a",
    active_prompt: prompt({ id: "prompt-1", target_agent_id: "agent-a" }),
    queued_prompts: [prompt({ id: "prompt-2", target_agent_id: "agent-a" })],
    agents: [
      agent("agent-a", { provider: "opencode" }),
      agent("agent-b", { provider: "claude" }),
    ],
  })
  const activeToolLabels = ["editing"]
  const controller = createAgentRuntimeProjectionController({
    getSession: () => session,
    getFocusedAgentId: () => "agent-a",
    getProviderRun: () => providerRun({ agent_instance_id: "agent-a" }),
    getVisibleTranscriptAgentId: () => "agent-a",
    getActiveToolLabels: () => activeToolLabels,
    getAgentPaneToolUpdates: () => [],
    getAgentPanePreviews: () => ({ "agent-a": "preview" }),
    getAgentActivityLabels: () => ({ "agent-a": "thinking" }),
    updateAgentActivityLabels: () => {},
    getAgentBusyLatches: () => ({}),
    updateAgentBusyLatches: () => {},
    getSubmitting: () => false,
    getSubmittingAgentId: () => null,
    getStreamingAgentId: () => null,
  })

  assert.equal(controller.agentPanePreview("agent-a"), "preview")
  assert.equal(controller.focusedAgent()?.id, "agent-a")
  assert.equal(controller.focusedBackendProvider(), "opencode")
  assert.equal(controller.focusedProviderRun()?.id, "run-1")
  assert.equal(controller.focusedActivePrompt()?.id, "prompt-1")
  assert.equal(controller.focusedQueueDepth(), 1)
  assert.equal(controller.anyPromptWork(), true)
  assert.deepEqual(controller.hasPromptWorkByAgent(), {
    "agent-a": true,
    "agent-b": false,
  })
  assert.equal(controller.activeToolLabelForAgent("agent-a"), "editing")
  assert.equal(controller.focusedActivityLabel(), "editing")
  assert.equal(controller.focusedAgentBusy(), true)
})

test("agent runtime projection queue depth follows shared runtime projection", () => {
  const session = runtimeSession({
    focused_agent_id: "agent-a",
    prompt_states: {
      "agent-a": {
        active_prompt: null,
        queued_prompts: [
          prompt({ id: "stale-1", target_agent_id: "agent-a" }),
          prompt({ id: "stale-2", target_agent_id: "agent-a" }),
        ],
      },
    },
    agent_activity: {
      "agent-a": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
    agents: [agent("agent-a")],
  })
  const controller = createAgentRuntimeProjectionController({
    getSession: () => session,
    getFocusedAgentId: () => "agent-a",
    getProviderRun: () => null,
    getVisibleTranscriptAgentId: () => "agent-a",
    getActiveToolLabels: () => [],
    getAgentPaneToolUpdates: () => [],
    getAgentPanePreviews: () => ({}),
    getAgentActivityLabels: () => ({}),
    updateAgentActivityLabels: () => {},
    getAgentBusyLatches: () => ({}),
    updateAgentBusyLatches: () => {},
    getSubmitting: () => false,
    getSubmittingAgentId: () => null,
    getStreamingAgentId: () => null,
  })

  assert.equal(controller.focusedQueueDepth(), 0)
  assert.equal(controller.agentQueuedDepth("agent-a"), 0)
})

test("agent runtime projection active prompt follows projected active turn identity", () => {
  const stalePrompt = prompt({ id: "prompt-stale", target_agent_id: "agent-a" })
  const session = runtimeSession({
    focused_agent_id: "agent-a",
    prompt_states: {
      "agent-a": {
        active_prompt: stalePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-a": {
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
    agents: [agent("agent-a")],
  })
  const controller = createAgentRuntimeProjectionController({
    getSession: () => session,
    getFocusedAgentId: () => "agent-a",
    getProviderRun: () => null,
    getVisibleTranscriptAgentId: () => "agent-a",
    getActiveToolLabels: () => [],
    getAgentPaneToolUpdates: () => [],
    getAgentPanePreviews: () => ({}),
    getAgentActivityLabels: () => ({}),
    updateAgentActivityLabels: () => {},
    getAgentBusyLatches: () => ({}),
    updateAgentBusyLatches: () => {},
    getSubmitting: () => false,
    getSubmittingAgentId: () => null,
    getStreamingAgentId: () => null,
  })

  assert.equal(controller.agentActivePrompt("agent-a"), null)
  assert.equal(controller.focusedActivePrompt(), null)
})

test("agent runtime projection updates busy latches and preserves active labels", () => {
  let latches: Record<string, boolean> = {}
  let labels: Record<string, string | null> = { "agent-a": "running" }
  const session = runtimeSession({
    focused_agent_id: "agent-a",
    agents: [agent("agent-a", { state: "Working" })],
  })
  const controller = createAgentRuntimeProjectionController({
    getSession: () => session,
    getFocusedAgentId: () => "agent-a",
    getProviderRun: () => null,
    getVisibleTranscriptAgentId: () => null,
    getActiveToolLabels: () => [],
    getAgentPaneToolUpdates: () => [],
    getAgentPanePreviews: () => ({}),
    getAgentActivityLabels: () => labels,
    updateAgentActivityLabels: (updater) => {
      labels = updater(labels)
    },
    getAgentBusyLatches: () => latches,
    updateAgentBusyLatches: (updater) => {
      latches = updater(latches)
    },
    getSubmitting: () => false,
    getSubmittingAgentId: () => null,
    getStreamingAgentId: () => null,
  })

  controller.markAgentBusy("agent-a")
  assert.deepEqual(latches, { "agent-a": true })
  controller.clearAgentBusy("agent-a")
  assert.deepEqual(latches, {})

  controller.setAgentActivityLabel("agent-a", null)
  assert.deepEqual(labels, { "agent-a": "running" })
  assert.deepEqual(controller.allAgentsBusyState(), [{ id: "agent-a", busy: true }])
})

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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

function prompt(overrides: Partial<PromptQueueItem> = {}): PromptQueueItem {
  return {
    id: "prompt",
    source_attachment_id: "attachment-1",
    target_agent_id: null,
    prompt: "hello",
    attachments: [],
    status: "queued",
    ...overrides,
  }
}

function providerRun(overrides: Partial<RuntimeProviderRun> = {}): RuntimeProviderRun {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: null,
    adapter_key: "opencode",
    provider: "opencode",
    account_profile: "default",
    model: "default",
    variant: null,
    usage_tokens_total: null,
    state: "running",
    ...overrides,
  }
}

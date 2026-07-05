import assert from "node:assert/strict"
import test from "node:test"

import type { AgentPromptState, WorkspaceLiveSyncStatus } from "./kernel-types.js"
import {
  sessionActivePromptIdForAgent,
  sessionActivePromptForAgent,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptStateForAgent,
} from "./session-prompt-identity.js"
import {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
import {
  sessionAgentIsBusy,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
} from "./session-prompt-work.js"
import {
  runtimeProviderRunForAgent,
  sessionActiveInteractionForAgent,
} from "./session-runtime-lookup.js"
import {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  resolveSessionStreamingAgentId,
  sessionFocusedAgentId,
  sessionRuntimeTransitionState,
  sessionShouldConfirmIdleTurnCompletion,
  sessionWorkingStateAfterPromptWork,
  shouldPreserveAgentActivityLabel,
  turnCompletionDelayMs,
} from "./session-runtime-transition.js"
import {
  agentRuntimeStateFromProjection,
  sessionAgentHasUnreadIdleOutput,
  sessionAgentPaneStatusBadge,
  sessionAgentRuntimeActivityProjection,
  sessionAgentRuntimeActivityStatus,
  sessionAgentRuntimeDisplayState,
  sessionAgentRuntimeState,
  sessionFocusedStatusBadge,
  sessionStatusLabel,
  sessionStatusMode,
} from "./session-runtime-status.js"
import {
  sessionAttachedFooterSummary,
  sessionFooterHint,
  sessionVisibleAgentSummary,
} from "./shell-session-footer.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"

test("sessionAgentIsBusy uses projected idle over stale legacy prompt state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-1"), false)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionAgentIsBusy treats missing projected agent activity as idle", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionAgentRuntimeActivityProjection returns normalized activity with idle fallback", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "external",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "turn-1",
          status: "running",
          phase: "streaming",
        },
      },
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionAgentRuntimeActivityProjection(session, "agent-1"), {
    status: "working",
    promptStatus: "running",
    busy: true,
    activeTurn: {
      prompt_id: "prompt-1",
      provider_run_id: "run-1",
      prompt_origin: "external",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
      status: "running",
      phase: "streaming",
    },
    activeTurnPromptId: "prompt-1",
    activeTurnProviderRunId: "run-1",
    activeTurnPromptOrigin: "external",
    activeTurnExternalProvider: "codex",
    activeTurnExternalProviderSessionId: "thread-1",
    activeTurnExternalProviderTurnId: "turn-1",
    activeTurnStatus: "running",
    activeTurnPhase: "streaming",
    activePromptCount: 1,
    activePromptCountExplicit: false,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: false,
    error: false,
    unreadIdleOutput: false,
  })
  assert.deepEqual(sessionAgentRuntimeActivityProjection(session, "agent-2"), {
    status: null,
    promptStatus: "none",
    busy: false,
    activeTurn: null,
    activePromptCount: 0,
    activePromptCountExplicit: false,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: false,
    error: false,
    unreadIdleOutput: false,
  })
  assert.equal(sessionAgentRuntimeActivityStatus(session, "agent-1"), "working")
  assert.equal(sessionAgentRuntimeActivityStatus(session, "agent-2"), "idle")
  assert.deepEqual(sessionAgentRuntimeActivityProjection(session, "agent-ghost"), {
    status: null,
    promptStatus: "none",
    busy: false,
    activeTurn: null,
    activePromptCount: 0,
    activePromptCountExplicit: false,
    queuedPromptCount: 0,
    queuedPromptCountExplicit: false,
    error: false,
    unreadIdleOutput: false,
  })
  assert.equal(sessionAgentRuntimeActivityStatus(session, "agent-ghost"), "idle")
  assert.equal(sessionAgentRuntimeActivityStatus(null, "agent-1"), "idle")
})

test("sessionFocusedStatusBadge projects detached, disconnected, focused, and multi-agent status", () => {
  assert.deepEqual(sessionFocusedStatusBadge({
    attached: false,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: false,
  }), badge([]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: true,
    activeStatusLabel: "reading",
    focusedBusy: true,
  }), badge([{ label: "DISCONNECTED", tone: "disconnected" }]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: false,
  }), badge([{ label: "IDLE", tone: "idle" }]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: "reading",
    focusedBusy: true,
  }), badge([{ label: "READING", tone: "working" }]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: true,
  }), badge([{ label: "THINKING", tone: "working" }]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: "patching",
    focusedBusy: true,
    agents: [
      { id: "agent-1", busy: false },
      { id: "agent-2", busy: true },
      { id: "agent-3", busy: false },
      { id: "agent-4", busy: true },
    ],
  }), badge([
    { label: "2 IDLE", tone: "idle" },
    { label: "2 WORKING", tone: "working" },
  ]))

  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: true,
    agents: [
      { id: "agent-1", busy: true },
      { id: "agent-2", busy: true },
    ],
  }), badge([{ label: "2 WORKING", tone: "working" }]))
})

test("sessionAgentPaneStatusBadge projects explicit activity and prompt work", () => {
  const idleAgent = { state: "Idle", is_processing: false }
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: null,
    activeLabel: null,
  }), { label: "", tone: "idle" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { ...idleAgent, state: "Error" },
    activeLabel: null,
  }), { label: "ERROR", tone: "error" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: idleAgent,
    activeLabel: "patching",
  }), { label: "PATCHING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: idleAgent,
    activeLabel: null,
    hasPromptWork: true,
  }), { label: "THINKING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: idleAgent,
    activeLabel: null,
    isStreaming: true,
  }), { label: "THINKING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: idleAgent,
    activeLabel: null,
    busyLatch: true,
  }), { label: "THINKING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { state: "Working", is_processing: true },
    activeLabel: null,
    useLegacyAgentProcessingState: false,
  }), { label: "IDLE", tone: "idle" })
})

test("session status mode and footer hint reflect prompt and queue work", () => {
  assert.equal(sessionStatusMode({
    daemonDisconnected: true,
    working: false,
    hasActivePrompt: false,
    submitting: false,
    queueDepth: 0,
  }), "disconnected")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActivePrompt: true,
    submitting: false,
    queueDepth: 0,
  }), "working")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActivePrompt: false,
    submitting: false,
    queueDepth: 1,
  }), "working")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActivePrompt: false,
    submitting: false,
    queueDepth: 0,
  }), "idle")

  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: "prompt-1",
    queueDepth: 2,
    statusLine: "Connected.",
  }), "Processing prompt-1; 2 queued.")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: "prompt-1",
    queueDepth: 0,
    statusLine: "Connected.",
  }), "Processing prompt-1.")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: null,
    queueDepth: 1,
    statusLine: "Connected.",
  }), "1 queued prompt.")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: null,
    queueDepth: 2,
    statusLine: "Connected.",
  }), "2 queued prompts.")
  assert.equal(sessionFooterHint({
    fatalError: "boom",
    activePromptId: "prompt-1",
    queueDepth: 0,
    statusLine: "Connected.",
  }), "boom")
  assert.equal(sessionFooterHint({
    fatalError: null,
    activePromptId: null,
    queueDepth: 0,
    statusLine: "Connected.",
  }), "Connected.")
})

test("sessionStatusLabel formats exact badge labels", () => {
  assert.equal(sessionStatusLabel("idle", "grepping"), "IDLE")
  assert.equal(sessionStatusLabel("disconnected", "grepping"), "DISCONNECTED")
  assert.equal(sessionStatusLabel("working", null), "THINKING")
  assert.equal(sessionStatusLabel("working", "grepping"), "GREPPING")
})

test("session attached footer summary projects visible agents, collaboration, prompt state, and sync", () => {
  assert.equal(sessionAttachedFooterSummary({
    session: makeSession({
      alias: "feature-refactor",
      agents: [
        makeAgent({ id: "agent-a", agent_ref: "main" }),
        makeAgent({ id: "agent-b", agent_ref: "review", alias: "QA", is_processing: true }),
      ],
    }),
    connectedClientCount: 2,
    multiAgentMode: true,
    sessionStatusMode: "working",
    hotkeyToggleLabel: "Ctrl+T",
  }), "Session feature-refactor • 2 CLIs connected • 2 visible agents • Ctrl+C to stop • Tab cycles focus • Ctrl+P opens workflow • Ctrl+T hotkeys")

  const sharedSession = makeSession({
    alias: "shared-review",
    agents: [makeAgent({ id: "agent-a" })],
    collaboration_agent_counts: {
      owned_agent_count: 1,
      other_user_agent_count: 3,
      total_agent_count: 4,
      collaborator_count: 2,
    },
  })
  const sharedSummary = sessionAttachedFooterSummary({
    session: sharedSession,
    connectedClientCount: 3,
    multiAgentMode: false,
    sessionStatusMode: "idle",
    hotkeyToggleLabel: "Ctrl+T",
  })
  assert.equal(sharedSummary, "Session shared-review • 3 CLIs connected • 1 visible agent • 3 collaborator agents • 2 collaborators • Ctrl+T hotkeys")
  assert.equal(sessionVisibleAgentSummary(sharedSession), "1 visible agent • 3 collaborator agents • 2 collaborators")
  assert.doesNotMatch(sharedSummary, /user-|agent-a|owner/)

  assert.equal(sessionAttachedFooterSummary({
    session: makeSession({ alias: "sync-review", agents: [makeAgent({ id: "agent-a" })] }),
    connectedClientCount: 1,
    multiAgentMode: false,
    sessionStatusMode: "idle",
    hotkeyToggleLabel: "Ctrl+T",
    workspaceLiveSyncStatus: workspaceLiveSyncStatus("conflict"),
  }), "Session sync-review • 1 CLI connected • 1 visible agent • sync managed conflict • Ctrl+T hotkeys")

  assert.equal(sessionAttachedFooterSummary({
    session: makeSession({ alias: "sync-off", agents: [makeAgent({ id: "agent-a" })] }),
    connectedClientCount: 1,
    multiAgentMode: false,
    sessionStatusMode: "idle",
    hotkeyToggleLabel: "Ctrl+T",
    workspaceLiveSyncStatus: workspaceLiveSyncStatus("off"),
  }), "Session sync-off • 1 CLI connected • 1 visible agent • sync off • Ctrl+T hotkeys")
})

test("sessionAgentRuntimeDisplayState maps unfocused unread idle output to done", () => {
  const session = makeSession({
    focused_agent_id: "agent-focused",
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
      "agent-focused": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
    },
  })

  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Done")
  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-focused"), false)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-focused",
    state: "Idle",
    is_processing: false,
  })), "Idle")
})

test("sessionFocusedAgentId keeps only session-scoped focus and falls back without explicit focus", () => {
  assert.equal(sessionFocusedAgentId(makeSession({
    focused_agent_id: "agent-2",
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")

  assert.equal(sessionFocusedAgentId(makeSession({
    focused_agent_id: "stale-agent",
    agents: [makeAgent({ id: "agent-1" })],
  })), null)

  assert.equal(sessionFocusedAgentId(makeSession({
    focused_agent_id: "stale-agent",
    agents: [],
  })), null)

  assert.equal(sessionFocusedAgentId(makeSession({
    focused_agent_id: null,
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-1")
})

test("sessionActiveInteractionForAgent returns active interaction scoped to agent", () => {
  const session = makeSession({
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-2",
      kind: "permission",
      level: "info",
      title: "Approve?",
      message: "Approve?",
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
      requested_at_ms: 1,
    }],
  })

  assert.equal(sessionActiveInteractionForAgent(session, "agent-2")?.id, "interaction-1")
  assert.equal(sessionActiveInteractionForAgent(session, "agent-1"), null)
  assert.equal(sessionActiveInteractionForAgent(session, null), null)
})

test("runtimeProviderRunForAgent returns provider run only for matching agent", () => {
  const run = {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "gpt-5.2",
    variant: null,
    usage_tokens_total: null,
    state: "running",
  }

  assert.equal(runtimeProviderRunForAgent(run, "agent-1")?.id, "run-1")
  assert.equal(runtimeProviderRunForAgent(run, "agent-2"), null)
  assert.equal(runtimeProviderRunForAgent(null, "agent-1"), null)
})

test("sessionPromptWorkSummary counts projected active turns and prompt state queues", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "Queued",
        }],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary prefers projected prompt counts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [{
          id: "stale-queued",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale queued",
          status: "Queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 2,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary ignores settled active turn statuses", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-1" }),
      makeAgent({ id: "agent-2" }),
      makeAgent({ id: "agent-3" }),
    ],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: malformedRuntimeValue("completed"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: malformedRuntimeValue(" Completed "),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-2": {
        status: "idle",
        prompt_status: malformedRuntimeValue("cancelled"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-3": {
        status: "idle",
        prompt_status: "settling",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-3",
          provider_run_id: "run-3",
          prompt_origin: "external",
          status: malformedRuntimeValue(" settling "),
          phase: "settling",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary counts prompt state active prompt for sparse busy activity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "none",
        busy: true,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary treats prompt states as runtime authority", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-3": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-3",
          source_attachment_id: "attach-3",
          target_agent_id: "agent-3",
          prompt: "queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-2", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-3", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 2,
  })
})

test("sessionPromptWorkByAgent honors prompt states across agents", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "review",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [
      makeAgent({ id: "agent-1" }),
      makeAgent({ id: "agent-2", is_processing: true, state: "Working" }),
    ],
  })

  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
  })
})

test("sessionPromptWorkByAgent prefers projected activity over stale prompt state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.deepEqual(sessionPromptWorkByAgent(session), {
    "agent-1": false,
    "agent-2": true,
  })
})

test("sessionProjectedStreamingAgentId uses projected activity before legacy active prompts", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")

  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    agent_activity: {},
    agents: [makeAgent({ id: "agent-1" })],
  })), null)
})

test("sessionProjectedStreamingAgentId resolves exactly one prompt-state active agent", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), "agent-2")

  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })), null)
})

test("sessionProjectedStreamingAgentId falls back to legacy active prompt without projections", () => {
  assert.equal(sessionProjectedStreamingAgentId(makeSession({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "legacy",
      status: "Running",
    },
    agents: [makeAgent({ id: "agent-1" })],
  })), "agent-1")
})

test("resolveSessionStreamingAgentId prefers processing, active prompt, then previous streaming agent", () => {
  const agents = [
    makeAgent({ id: "agent-a", is_processing: false, state: "Idle" }),
    makeAgent({ id: "agent-b", is_processing: true, state: "Working" }),
  ]

  assert.equal(resolveSessionStreamingAgentId(agents, "agent-a", true, true, "agent-a"), "agent-b")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], "agent-a", true, false, null), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, true, false, "agent-a"), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, false, true, "agent-a"), "agent-a")
  assert.equal(resolveSessionStreamingAgentId([makeAgent({ id: "agent-a" })], null, false, false, "agent-a"), null)
})

test("resolveSessionStreamingAgentId can ignore legacy processing for projected sessions", () => {
  const agents = [
    makeAgent({ id: "agent-a", is_processing: true, state: "Working" }),
    makeAgent({ id: "agent-b", is_processing: false, state: "Idle" }),
  ]

  assert.equal(resolveSessionStreamingAgentId(agents, "agent-b", true, false, null, false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, true, false, "agent-b", false), "agent-b")
  assert.equal(resolveSessionStreamingAgentId(agents, null, false, true, "agent-b", false), null)
  assert.equal(resolveSessionStreamingAgentId(agents, null, false, false, null, false), null)
})

test("sessionRuntimeTransitionState preserves active labels and clears idle labels", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })
  const nextSession = makeSession({
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-2", state: "Working", is_processing: true }),
    ],
    active_prompt: {
      id: "prompt-2",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "run",
      status: "Running",
    },
    focused_agent_id: "agent-2",
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: {
      "agent-1": "thinking",
      "agent-2": "writing",
    },
  }), {
    nextFocusedAgentId: "agent-2",
    nextHasPromptWork: true,
    nextStreamingAgentId: "agent-2",
    nextFocusedActivityLabel: "writing",
    nextAgentActivityLabels: {
      "agent-1": null,
      "agent-2": "writing",
    },
    nextWorking: true,
    previousAgentSignature: "agent-1,agent-2",
    nextAgentSignature: "agent-1,agent-2",
  })
})

test("sessionRuntimeTransitionState clears stale streaming when projected activity is idle", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  }), {
    nextFocusedAgentId: "agent-1",
    nextHasPromptWork: false,
    nextStreamingAgentId: null,
    nextFocusedActivityLabel: null,
    nextAgentActivityLabels: {
      "agent-1": null,
    },
    nextWorking: true,
    previousAgentSignature: "agent-1",
    nextAgentSignature: "agent-1",
  })
})

test("sessionRuntimeTransitionState does not stream queued-only projected activity", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "queued",
        busy: true,
        active_prompt_count: 0,
        queued_prompt_count: 1,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  }), {
    nextFocusedAgentId: "agent-1",
    nextHasPromptWork: true,
    nextStreamingAgentId: null,
    nextFocusedActivityLabel: "thinking",
    nextAgentActivityLabels: {
      "agent-1": "thinking",
    },
    nextWorking: true,
    previousAgentSignature: "agent-1",
    nextAgentSignature: "agent-1",
  })
})

test("sessionRuntimeTransitionState resolves streaming from prompt state before stale processing", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })
  const nextSession = makeSession({
    agents: [
      makeAgent({ id: "agent-1", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-2", state: "Idle", is_processing: false }),
    ],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "run",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: { "agent-1": "thinking", "agent-2": "writing" },
  })

  assert.equal(transition.nextStreamingAgentId, "agent-2")
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-1": null,
    "agent-2": "writing",
  })
})

test("sessionRuntimeTransitionState treats empty prompt states as authoritative idle", () => {
  const currentSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const nextSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    prompt_states: {},
  })

  const transition = sessionRuntimeTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-1",
    currentAgentActivityLabels: { "agent-1": "thinking" },
  })

  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.nextStreamingAgentId, null)
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-1": null,
  })
})

test("sessionWorkingStateAfterPromptWork keeps working latched until completion is confirmed", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "run",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  })

  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: true,
    nextSession: idleSession,
  }), true)
  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: true,
    nextSession: activeSession,
  }), true)
  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: false,
    nextSession: activeSession,
  }), true)
  assert.equal(sessionWorkingStateAfterPromptWork({
    currentWorking: false,
    nextSession: idleSession,
  }), false)
})

test("agent busy latches set, clear, and preserve unchanged records", () => {
  const empty: Record<string, boolean> = {}
  assert.equal(readAgentBusyLatch(empty, null), false)
  assert.equal(nextAgentBusyLatches(empty, null, true), empty)

  const busy = nextAgentBusyLatches(empty, "agent-1", true)
  assert.deepEqual(busy, { "agent-1": true })
  assert.equal(readAgentBusyLatch(busy, "agent-1"), true)
  assert.equal(nextAgentBusyLatches(busy, "agent-1", true), busy)

  const cleared = nextAgentBusyLatches(busy, "agent-1", false)
  assert.deepEqual(cleared, {})
})

test("agent activity labels preserve current labels only while activity is still authoritative", () => {
  const current = { "agent-1": "writing" }
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", "reading", false), { "agent-1": "reading" })
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", null, true), { "agent-1": "writing" })
  assert.deepEqual(nextAgentActivityLabels(current, "agent-1", null, false), { "agent-1": null })
  assert.equal(nextAgentActivityLabels(current, null, "reading", false), current)
})

test("agent activity labels are preserved for streaming, prompt work, and working agents", () => {
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1" })] }),
    streamingAgentId: "agent-1",
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({
      agents: [makeAgent({ id: "agent-1" })],
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
    }),
    streamingAgentId: null,
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1", state: "Working" })] }),
    streamingAgentId: null,
  }), true)
  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1" })] }),
    streamingAgentId: null,
  }), false)
})

test("projected idle activity suppresses stale legacy busy state", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Working", is_processing: true })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(shouldPreserveAgentActivityLabel({
    agentId: "agent-1",
    session,
    streamingAgentId: null,
  }), false)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session,
    streamingAgentId: null,
    agentActivityLabels: {},
    agentBusyLatches: {},
  }), [{ id: "agent-1", busy: false }])
})

test("focused activity and busy state derive from labels, latches, prompt work, and agent state", () => {
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "agent-1",
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), "reading")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: "agent-1",
    activeToolLabel: null,
    agentActivityLabel: "thinking",
  }), "thinking")
  assert.equal(deriveFocusedActivityLabel({
    focusedAgentId: null,
    activeToolLabel: "reading",
    agentActivityLabel: "thinking",
  }), null)

  const idleSession = makeSession({ agents: [makeAgent({ id: "agent-1" })] })
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: idleSession,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: { "agent-1": true },
  }), true)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: makeSession({ agents: [makeAgent({ id: "agent-1", is_processing: true })] }),
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), true)
  assert.equal(deriveFocusedAgentBusy({
    focusedAgentId: "agent-1",
    submitting: false,
    submittingAgentId: null,
    session: idleSession,
    streamingAgentId: null,
    focusedActivityLabel: null,
    agentBusyLatches: {},
  }), false)
})

test("active tool labels prefer visible transcript tools and ignore completed pane tools", () => {
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-1",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading", "patching"],
    agentPaneToolUpdates: null,
  }), "patching")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-2",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: [
      { tool: "read", status: "completed" },
      { tool: "bash", status: "running" },
      { tool: "edit", status: "error" },
      { tool: "grep", status: "cancelled" },
    ],
  }), "bashing")
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: null,
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: ["reading"],
    agentPaneToolUpdates: null,
  }), null)
  assert.equal(resolveActiveToolLabelForAgent({
    agentId: "agent-2",
    visibleTranscriptAgentId: "agent-1",
    activeToolLabels: [],
    agentPaneToolUpdates: [{ tool: "custom_tool", status: "running" }],
    toolActivityLabel: (tool?: string | null) => tool ? `custom ${tool}` : null,
  }), "custom custom_tool")
})

test("all agent busy state is derived per agent", () => {
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: true,
    submittingAgentId: "agent-1",
    session: makeSession({ agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2", state: "Working" })] }),
    streamingAgentId: null,
    agentActivityLabels: {},
    agentBusyLatches: {},
  }), [
    { id: "agent-1", busy: true },
    { id: "agent-2", busy: true },
  ])
  assert.deepEqual(deriveAllAgentsBusyState({
    submitting: false,
    submittingAgentId: null,
    session: makeSession({ agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })] }),
    streamingAgentId: "agent-2",
    agentActivityLabels: { "agent-1": "thinking" },
    agentBusyLatches: {},
  }), [
    { id: "agent-1", busy: true },
    { id: "agent-2", busy: true },
  ])
})

test("sessionShouldConfirmIdleTurnCompletion treats idle snapshots as stale-turn completion", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Focused" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionHasPromptWork(idleSession), false)
  assert.equal(sessionHasProcessingAgent(idleSession), false)
  assert.equal(sessionShouldConfirmIdleTurnCompletion({
    nextSession: idleSession,
    currentWorking: true,
    currentSubmitting: false,
    currentBusyLatches: {},
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), true)
})

test("sessionShouldConfirmIdleTurnCompletion does not override active prompt or processing snapshots", () => {
  const activePromptSession = makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "running",
    },
    agents: [makeAgent({ id: "agent-1", is_processing: false, state: "Focused" })],
  })
  const processingSession = makeSession({
    agents: [makeAgent({ id: "agent-1", is_processing: true, state: "Working" })],
  })

  for (const nextSession of [activePromptSession, processingSession]) {
    assert.equal(sessionShouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: true,
      currentSubmitting: true,
      currentBusyLatches: { "agent-1": true },
      currentStreamingAgentId: "agent-1",
      currentProviderActivityLabel: "thinking",
      currentActiveStatusLabel: "thinking",
    }), false)
  }
})

test("turnCompletionDelayMs waits for prompt work and terminal record flushes", () => {
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(turnCompletionDelayMs({
    session: activeSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 1,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: true,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), null)
})

test("turnCompletionDelayMs returns the remaining quiet window after last turn activity", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 900,
    now: 1_000,
    quietWindowMs: 1_500,
  }), 1_400)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 0,
    now: 1_500,
    quietWindowMs: 1_500,
  }), 0)
  assert.equal(turnCompletionDelayMs({
    session: idleSession,
    pendingTerminalRecordCount: 0,
    pendingTerminalRecordFlush: false,
    lastTurnActivityAt: 2_000,
    now: 1_000,
    quietWindowMs: 1_500,
  }), 1_500)
})

test("sessionPromptWorkSummary ignores prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionAgentRuntimeState normalizes projected error status", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: malformedRuntimeValue(" Error "),
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Error")

  assert.equal(agentRuntimeStateFromProjection(makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  }), {
    agentActivity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        error: true,
      },
    },
  }), "Error")
})

test("sessionHasActivePrompt follows projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Working")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-2")
})

test("session prompt helpers ignore settled projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionPromptForAgent rejects legacy prompts that do not match projected active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt does not invent prompt identity from anonymous projected activity", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers use prompt state identity when projected activity is busy without active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("sessionHasActivePrompt follows projected active turn even when prompt state is absent", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt falls back to legacy fields when projection is unavailable", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level active prompt", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent normalizes prompt states with omitted queued prompts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
      } as AgentPromptState,
    },
  })

  assert.deepEqual(sessionPromptStateForAgent(session, "agent-1"), {
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [],
  })
})

test("session prompt helpers treat missing prompt state agents as idle", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    prompt_states: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent ignores legacy prompts once activity projection exists", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionPromptStateForAgent(session, "agent-1"), null)
})

test("sessionPromptStateForAgent scopes legacy top-level prompts by agent", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "queued",
      status: "Queued",
    }, {
      id: "queued-other",
      source_attachment_id: "attach-2",
      target_agent_id: "agent-2",
      prompt: "other",
      status: "Queued",
    }],
  })

  assert.deepEqual(sessionPromptStateForAgent(session, "agent-1"), {
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "running",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-1",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "queued",
      status: "Queued",
    }],
  })
  assert.equal(sessionPromptStateForAgent(session, "agent-2")?.active_prompt, null)
  assert.equal(sessionPromptStateForAgent(session, null), null)
})

test("session prompt helpers ignore prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    ],
  })

  assert.equal(sessionAgentIsBusy(session, "agent-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "prompt-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "queued-ghost"), false)
  assert.equal(sessionPromptForAgent(session, "agent-ghost"), null)
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level queued prompts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers ignore top-level prompts for other agents", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-other"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionActivePromptLifecycleRecords uses projected active turns and deterministic order", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-b", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-a", state: "Working", is_processing: true }),
    ],
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "prompt-a-stale",
          source_attachment_id: "attach-a",
          target_agent_id: "agent-a",
          prompt: "stale",
          status: "Running",
          prompt_origin: "arroba",
        },
        queued_prompts: [],
      },
      "agent-b": {
        active_prompt: {
          id: "prompt-b-state",
          source_attachment_id: "attach-b",
          target_agent_id: "agent-b",
          prompt: "running",
          status: "Running",
          prompt_origin: "external",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-a-live",
          status: "running",
          prompt_origin: "external",
          phase: "streaming",
        },
      },
      "agent-b": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "prompt-a-live",
    status: "running",
    promptOrigin: "external",
    target_agent_id: "agent-a",
  }, {
    id: "prompt-b-state",
    source_attachment_id: "attach-b",
    target_agent_id: "agent-b",
    prompt: "running",
    status: "running",
    prompt_origin: "external",
    promptOrigin: "external",
  }])
})

test("sessionActivePromptIdForAgent prefers projected active turn and per-agent prompt state", () => {
  assert.equal(sessionActivePromptIdForAgent(makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "projected-prompt",
          status: "running",
          phase: "streaming",
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "projected-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "state-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-idle-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), null)
})

test("sessionActivePromptForAgent returns only the active prompt under projected runtime state", () => {
  assert.equal(sessionActivePromptForAgent(makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "queued",
        }],
      },
    },
  }), "agent-1"), null)

  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  }), "agent-1"), null)

  const activePrompt = {
    id: "prompt-active",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "active",
    status: "running",
  }
  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
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
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  }), "agent-1")?.id, "prompt-active")

  assert.equal(sessionActivePromptForAgent(makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "running",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          ...activePrompt,
          id: "prompt-other",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  }), "agent-1"), null)
})

test("sessionActivePromptIdForAgent falls back to prompt state for sparse busy activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), "state-prompt")
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "state-prompt")
})

test("sessionActivePromptIdForAgent ignores legacy active prompt for sparse activity without prompt state", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale top-level prompt",
      status: "running",
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
})

test("sessionActivePromptIdForAgent suppresses prompt state for idle or missing projected activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-a",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "stale-b",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-2"), null)
})

test("sessionActivePromptIdForAgent ignores settled projected active turn identity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-settled",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, null), null)
})

test("sessionActivePromptLifecycleRecords treats projected idle as authoritative", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "cancelling",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "cancelling",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [])
})

test("sessionActivePromptLifecycleRecords falls back to legacy active prompt without projections", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "legacy",
      status: "Running",
      prompt_origin: " External ",
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "prompt-legacy",
    source_attachment_id: "attach-1",
    target_agent_id: "agent-1",
    prompt: "legacy",
    status: "running",
    prompt_origin: " External ",
    promptOrigin: "external",
  }])
})

test("sessionPromptLifecycleTransition detects when a cancelling prompt settles", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "cancelling",
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition normalizes cancelling prompt status", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: " Cancelling ",
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition treats projected idle activity as prompt settlement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "cancelling",
      },
    }),
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "stale",
        status: "cancelling",
      },
      agent_activity: {},
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition ignores already-settled projected active turns", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-settled",
            status: malformedRuntimeValue("cancelled"),
            phase: malformedRuntimeValue("settled"),
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, false)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, [])
})

test("sessionPromptLifecycleTransition detects normal prompt replacement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "running",
      },
    }),
    makeSession({
      active_prompt: {
        id: "prompt-2",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "next",
        status: "running",
      },
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition settles external prompts when they disappear", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "running",
            prompt_origin: " External ",
            phase: "streaming",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition settles cancelling external prompts", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "cancelling",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "cancelling",
            prompt_origin: "External",
            phase: "settling",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

function badge(parts: Array<{ label: string; tone: "idle" | "working" | "disconnected" | "error" }>) {
  return {
    label: parts.map((part) => part.label).join(" "),
    tone: parts.some((part) => part.tone === "working")
      ? "working"
      : parts[0]?.tone ?? "idle",
    parts,
  }
}

function malformedRuntimeValue<T>(value: string): T {
  return value as unknown as T
}

function workspaceLiveSyncStatus(
  footerState: WorkspaceLiveSyncStatus["footer_state"],
): WorkspaceLiveSyncStatus {
  return {
    session_id: "session-1",
    mode: footerState === "off" ? "unrestricted" : "managed",
    footer_state: footerState,
    sync_groups: [],
    targets: [],
    conflicts: [],
    ignore: {
      rules: [],
      force_excludes: [],
    },
  }
}

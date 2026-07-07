import assert from "node:assert/strict"
import test from "node:test"

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
  sessionWorkingStateAfterTurnWork,
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
import { badge, workspaceLiveSyncStatus } from "./session-runtime-projection.test-support.js"

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

test("session status mode and footer hint reflect active turn work and queued prompts", () => {
  assert.equal(sessionStatusMode({
    daemonDisconnected: true,
    working: false,
    hasActiveTurnWork: false,
    submitting: false,
    queueDepth: 0,
  }), "disconnected")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: true,
    submitting: false,
    queueDepth: 0,
  }), "working")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: false,
    submitting: false,
    queueDepth: 1,
  }), "idle")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: false,
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

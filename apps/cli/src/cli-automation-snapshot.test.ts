import assert from "node:assert/strict"
import test from "node:test"

import { buildCliAutomationSnapshot } from "./cli-automation-snapshot.js"
import type { ShellContext } from "@arroba/kernel-client/shell-core"
import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room-state.js"

test("buildCliAutomationSnapshot projects session and interaction state for automation", () => {
  const catalog = fallbackProviderCatalog()
  const agent = {
    id: "agent-1",
    agent_ref: "A",
    alias: "frontend",
    provider: "opencode",
    model: "default",
    state: "Idle",
    is_processing: false,
  } as AgentInstance
  const session = {
    id: "session-1",
    workspace_id: "/repo",
    worktree_id: "/repo",
    focused_agent_id: "agent-1",
    agents: [agent],
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-1",
      kind: "choice",
      level: "info",
      title: "Approve",
      message: "Continue?",
      timeout_sec: null,
      default_on_timeout: null,
      requested_at_ms: 1,
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
    }],
    workflows: [{ id: "workflow-1", alias: "release", nodes: [], edges: [], endpoints: [] }],
    workflow_runs: [],
  } as unknown as RuntimeSession

  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "workflow",
    workflowScreenActive: () => true,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => session,
    focusedAgentId: () => "agent-1",
    agentActivityLabels: () => ({ "agent-1": "reviewing" }),
    hasPromptWorkByAgent: () => ({ "agent-1": true }),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => true,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => "workflow-1",
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => "agent-1",
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    interactionChoiceSelection: () => 1,
    interactionCustomReply: () => "ship it",
    interactionCustomEditing: () => true,
  })

  assert.equal(snapshot.screen, "workflow")
  assert.equal((snapshot.session as { id: string }).id, "session-1")
  assert.equal(snapshot.transcript.visibleAgentId, "agent-1")
  assert.equal((snapshot.selectedWorkflow as { alias: string }).alias, "release")
  assert.deepEqual((snapshot.interactions as Array<Record<string, unknown>>)[0], {
    id: "interaction-1",
    agentId: "agent-1",
    kind: "choice",
    level: "info",
    title: "Approve",
    message: "Continue?",
    timeoutSec: null,
    defaultOnTimeout: null,
    focused: true,
    selectedChoiceIndex: 1,
    customChoice: null,
    customReply: "ship it",
    customEditing: true,
    choices: [{ id: "yes", label: "Yes", style: "primary" }],
  })
})

test("buildCliAutomationSnapshot exposes external transcript and queued prompt metadata", () => {
  const catalog = fallbackProviderCatalog()
  const agent = {
    id: "agent-1",
    agent_ref: "A",
    alias: "worker",
    provider: "opencode",
    model: "default",
    state: "Idle",
    is_processing: false,
  } as AgentInstance
  const session = {
    id: "session-1",
    workspace_id: "/repo",
    worktree_id: "/repo",
    focused_agent_id: "agent-1",
    agents: [agent],
    active_interactions: [],
    workflows: [],
    workflow_runs: [],
  } as unknown as RuntimeSession

  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "agents",
    workflowScreenActive: () => false,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => session,
    focusedAgentId: () => "agent-1",
    agentActivityLabels: () => ({}),
    hasPromptWorkByAgent: () => ({}),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => true,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => null,
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    visibleTranscriptAgentId: () => "agent-1",
    transcriptEntries: () => [{
      id: 1,
      role: "assistant",
      text: "external output",
      source: "external_provider_observed",
      externalProvider: "opencode",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 123,
    }],
    agentPaneEntries: () => ({
      "agent-1": [{
        id: 2,
        role: "notice",
        text: "queued behind external turn",
        queuedPrompt: {
          promptId: "prompt-1",
          agentId: "agent-1",
          status: "queued",
          steerDisabled: true,
        },
      }],
    }),
    footerFlash: () => null,
    interactionChoiceSelection: () => 0,
    interactionCustomReply: () => "",
    interactionCustomEditing: () => false,
  })

  assert.deepEqual((snapshot.transcript?.entries as Array<Record<string, unknown>>)[0], {
    id: 1,
    role: "assistant",
    text: "external output",
    queuedPrompt: null,
    source: "external_provider_observed",
    externalProvider: "opencode",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 123,
    turnId: null,
    hidden: false,
    blobCollapsible: false,
    blobCollapsed: null,
    blobTitle: null,
    blobSummary: null,
    historyBlobId: null,
    historyBlobAgentId: null,
    historyBlobSourceId: null,
    historyBlobSourceAgentId: null,
    historyBlobLoaded: null,
    historyBlobLoading: null,
    historyBlobError: null,
  })
  assert.deepEqual((snapshot.agentPanes?.["agent-1"] as Array<Record<string, unknown>>)[0]?.queuedPrompt, {
    promptId: "prompt-1",
    agentId: "agent-1",
    status: "queued",
    steerDisabled: true,
  })
})

test("buildCliAutomationSnapshot exposes waiting room unattached agent rows", () => {
  const catalog = fallbackProviderCatalog()
  const snapshot = buildCliAutomationSnapshot({
    workspaceScreenMode: () => "agents",
    workflowScreenActive: () => false,
    daemonDisconnected: () => false,
    statusLine: () => "ready",
    sessionState: () => ({
      id: "detached",
      workspace_id: "/repo",
      worktree_id: "/repo",
      focused_agent_id: null,
      agents: [],
      active_interactions: [],
      workflows: [],
      workflow_runs: [],
    }) as unknown as RuntimeSession,
    focusedAgentId: () => null,
    agentActivityLabels: () => ({}),
    hasPromptWorkByAgent: () => ({}),
    streamingAgentId: () => null,
    agentBusyLatch: () => false,
    isAttached: () => false,
    waitingRoomState: () => createWaitingRoomState([], catalog, "opencode", "default", "", "opencode", DEFAULT_THEME_REGISTRY),
    availableSessions: () => [],
    providerCatalogState: () => catalog,
    waitingRoomCloudNotice: () => null,
    waitingRoomInventoryStatus: () => "ready",
    relayStatusState: () => null,
    remoteMachinesState: () => [],
    remoteKernelsState: () => [],
    terminalsState: () => [],
    externalProviderSessionsState: () => [{
      external_session_id: "opencode:thread-1",
      provider: "opencode",
      provider_session_id: "thread-1",
      title: "External OpenCode thread",
      first_prompt_preview: "external prompt",
      last_modified_at_ms: 1_700_000_000_000,
    }],
    externalProviderSessionsPageState: () => ({ hasMore: false, nextCursor: null }),
    slicesState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => null,
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => null,
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    interactionChoiceSelection: () => 0,
    interactionCustomReply: () => "",
    interactionCustomEditing: () => false,
  })

  const rows = (snapshot.waitingRoom as { rows: Array<Record<string, unknown>> }).rows
  assert.deepEqual(rows.find((row) => row.id === "external-session:opencode:thread-1"), {
    id: "external-session:opencode:thread-1",
    externalSessionId: "opencode:thread-1",
    title: "External OpenCode thread",
    value: "opencode",
    focused: false,
    selectable: true,
  })
})

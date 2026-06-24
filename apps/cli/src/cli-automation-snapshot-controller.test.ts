import assert from "node:assert/strict"
import test from "node:test"

import type { ShellContext } from "@arroba/kernel-client/shell-core"

import { createCliAutomationSnapshotController } from "./cli-automation-snapshot-controller.js"
import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room-state.js"

test("CLI automation snapshot controller normalizes interaction state defaults", () => {
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
    workflows: [],
    workflow_runs: [],
  } as unknown as RuntimeSession

  const controller = createCliAutomationSnapshotController({
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
    transcriptEntries: () => [],
    visibleTranscriptAgentId: () => "agent-1",
    agentPaneEntries: () => ({}),
    footerFlash: () => null,
    getInteractionChoiceSelection: () => undefined,
    getInteractionCustomReply: () => null,
    isInteractionCustomEditing: () => false,
  })

  const snapshot = controller.snapshot()
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
    selectedChoiceIndex: 0,
    customChoice: null,
    customReply: "",
    customEditing: false,
    choices: [{ id: "yes", label: "Yes", style: "primary" }],
  })
})

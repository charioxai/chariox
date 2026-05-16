import assert from "node:assert/strict"
import test from "node:test"

import { buildCliAutomationSnapshot } from "./cli-automation-snapshot.js"
import type { ShellContext } from "@arroba/kernel-client/shell-core"
import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room.js"

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
    slicesState: () => [],
    waitingRoomTargets: () => ({ workspacePath: "/repo", worktreePath: "/repo" }),
    themeRegistryState: () => DEFAULT_THEME_REGISTRY,
    selectedWorkflowId: () => "workflow-1",
    selectedWorkflowNodeId: () => null,
    workspaceShellContext: () => ({ cwd: "/repo", env: {} }) as unknown as ShellContext,
    workspaceShellEntries: () => [],
    footerFlash: () => null,
    interactionChoiceSelection: () => 1,
    interactionCustomReply: () => "ship it",
    interactionCustomEditing: () => true,
  })

  assert.equal(snapshot.screen, "workflow")
  assert.equal((snapshot.session as { id: string }).id, "session-1")
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

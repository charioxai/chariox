import assert from "node:assert/strict"
import test from "node:test"
import { createRoot } from "solid-js"
import { createCliRuntimeProjectionComposition } from "./cli-runtime-projection-composition.js"

test("MP-08 / MP-10 / MP-11 workflow permission input follows the selected agent", () => {
  let mode = "workflow"
  const permissionA = { id: "permission-a", agent_id: "agent-a" }
  const permissionB = { id: "permission-b", agent_id: "agent-b" }
  const state = {
    id: "session", agents: [{ id: "agent-a" }, { id: "agent-b" }],
    focused_agent_id: "agent-b", active_interactions: [permissionB],
    workflows: [{ id: "workflow-a", nodes: [{ id: "node-a", agent_id: "agent-a" }] }],
    workflow_runs: [], attachment_ids: [],
  }
  const noop = () => {}
  createRoot(dispose => {
    try {
      const projection = createCliRuntimeProjectionComposition({
        appLogger: { debug: noop, info: noop, warn: noop, error: noop },
        debugLogsEnabled: false,
        attachmentState: () => ({ id: "attached" }),
        sessionState: () => state,
        workspaceScreenMode: () => mode,
        multiAgentResponseLayout: () => "split", maxAgentsPerScreen: () => 2,
        workflowScreenActive: () => mode === "workflow",
        selectedWorkflowId: () => "workflow-a", selectedWorkflowNodeId: () => "node-a",
        providerRunState: () => null,
        primaryTranscriptRuntimeStore: { activeToolLabelValues: () => [] },
        agentPaneRuntimeStore: { toolUpdatesForAgent: () => [] },
        agentPanePreviews: () => new Map(), agentActivityLabels: () => new Map(),
        setAgentActivityLabels: noop, agentBusyLatches: () => new Map(),
        setAgentBusyLatches: noop, submitting: () => false,
        promptSubmissionAgentStateController: { getSubmittingAgentId: () => null },
        streamingAgentId: () => null, entries: () => [], daemonDisconnected: () => false,
        transcriptScrollboxRefController: { hasScrollbox: () => false },
      } as never)
      assert.equal(projection.workflowPromptState().selectedAgent?.id, "agent-a")
      assert.equal(projection.focusedAgentInteraction(), null,
        "a hidden permission for agent-b must not consume workflow agent-a's input")
      state.active_interactions = [permissionB, permissionA]
      assert.equal(projection.focusedAgentInteraction()?.id, "permission-a")
      mode = "agents"
      assert.equal(projection.focusedAgentInteraction()?.id, "permission-b")
    } finally { dispose() }
  })
})

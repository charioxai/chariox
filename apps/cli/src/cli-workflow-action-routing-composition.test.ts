import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
} from "./cli-types.js"
import { createCliAppWorkflowActionComposition } from "./cli-app-workflow-composition.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"

test("workflow capabilities survive app and command action composition", async () => {
  const requests: Record<string, any>[] = []
  const footers: string[] = []
  const notices: string[] = []
  let selectedWorkflowId: string | null = "workflow-1"
  let workspaceScreenMode = "workflow"
  let state = session({ workflows: [workflow()] })
  const queuedPrompt = workflowQueuedPrompt()
  const queue = workflowQueue()
  const run = workflowRun()

  const client = {
    send: async (request: Record<string, any>): Promise<Record<string, unknown>> => {
      requests.push(request)
      if (request.PauseWorkflowRun) {
        return { WorkflowRunPaused: { workflow_run: { ...run, status: "Paused" }, session: state } }
      }
      if (request.GetWorkflowRun) {
        return { WorkflowRun: { workflow_run: run } }
      }
      if (request.ListWorkflowPromptQueues) {
        return { WorkflowPromptQueuesListed: { queues: [queue] } }
      }
      if (request.ListQueuedWorkflowPrompts) {
        return { QueuedWorkflowPromptsListed: { queued_prompts: [queuedPrompt] } }
      }
      if (request.CreateWorkflowPromptQueue) {
        return { WorkflowPromptQueueCreated: { queue, session: state } }
      }
      if (request.UpdateWorkflowPromptQueue) {
        const body = request.UpdateWorkflowPromptQueue
        return {
          WorkflowPromptQueueUpdated: {
            queue: {
              ...queue,
              alias: body.alias ?? queue.alias,
              priority: body.priority ?? queue.priority,
              enabled: body.enabled ?? queue.enabled,
            },
            session: state,
          },
        }
      }
      if (request.UpdateQueuedWorkflowPrompt) {
        return { QueuedWorkflowPromptUpdated: { queued_prompt: queuedPrompt, session: state } }
      }
      if (request.RemoveQueuedWorkflowPrompt) {
        return { QueuedWorkflowPromptRemoved: { queued_prompt: queuedPrompt, session: state } }
      }
      if (request.ClearWorkflowPromptQueue) {
        return { WorkflowPromptQueueCleared: { queued_prompts: [queuedPrompt], session: state } }
      }
      if (request.RemoveWorkflowPromptQueue) {
        return { WorkflowPromptQueueRemoved: { queue, session: state } }
      }
      if (request.BindWorkflowEndpoint) {
        const body = request.BindWorkflowEndpoint
        const endpoint = { id: "endpoint-1", alias: "start", entry_node_id: body.entry_node_id }
        return { WorkflowEndpointBound: { endpoint, workflow: workflow({ endpoints: [endpoint] }), session: state } }
      }
      if (request.ResolveWorkflow) {
        return { WorkflowResolved: { workflow: state.workflows?.[0] ?? workflow() } }
      }
      if (request.ApplyWorkflowDesignOp) {
        const op = request.ApplyWorkflowDesignOp.op
        if (op.kind === "endpoint_update") {
          state = session({
            workflows: [workflow({
              endpoints: [{
                ...workflow().endpoints![0]!,
                ...(op.patch.alias !== undefined ? { alias: op.patch.alias } : {}),
                ...(op.patch.entry_node_id !== undefined ? { entry_node_id: op.patch.entry_node_id } : {}),
              }],
            })],
          })
        } else if (op.kind === "endpoint_remove") {
          state = session({ workflows: [workflow({ endpoints: [] })] })
        } else if (op.kind === "workflow_remove") {
          state = session({ workflows: [] })
        }
        return { WorkflowDesignOpAccepted: { session: state } }
      }
      throw new Error(`unexpected request ${Object.keys(request)[0] ?? "unknown"}`)
    },
  }

  const workflowActions = createCliAppWorkflowActionComposition({
    client,
    originClientId: "cli-1",
    bindWorkflowNodeInstructionsEditor: () => {},
    workflowNodeInstructionsEditor: () => null,
    setWorkflowNodeInstructionsEditor: () => {},
    workflowScreenShowing: () => workspaceScreenMode === "workflow",
    setWorkspaceScreenMode: (mode: string) => { workspaceScreenMode = mode },
    rebuildTranscript: () => {},
    scheduleTimer: () => 0,
    focusPromptInput: () => {},
    setWorkflowInspectorMode: () => {},
    setSelectedWorkflowId: (workflowId: string | null) => { selectedWorkflowId = workflowId },
    isAttached: () => true,
    sessionState: () => state,
    applySessionState: (nextSession: RuntimeSession) => { state = nextSession },
    selectedWorkflowId: () => selectedWorkflowId,
    selectedWorkflowNodeId: () => null,
    setSelectedWorkflowNodeId: () => {},
    setSelectedWorkflowComponent: () => {},
    workspaceScreenMode: () => workspaceScreenMode,
    applyResponseLayout: () => {},
  })

  const handlers = createCliCommandActionComposition({
    ...workflowActions,
    client,
    options: { clientId: "cli-1", accountProfile: "default", model: "default", effort: "", provider: "opencode" },
    preferencesState: () => ({}),
    setPreferencesState: () => {},
    initialWorkspaceTarget: "workspace-1",
    initialWorktreeTarget: "worktree-1",
    pendingWorkspaceTarget: () => "workspace-1",
    pendingWorktreeTarget: () => "worktree-1",
    setPendingWorkspaceTarget: () => {},
    setPendingWorktreeTarget: () => {},
    isAttached: () => true,
    sessionState: () => state,
    attachmentState: () => ({ id: "attachment-1", session_id: state.id }),
    providerRunState: () => null,
    currentModelId: () => "default",
    currentVariantId: () => "medium",
    focusedAgentId: () => null,
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message: string, tone: string) => footers.push(`${tone}:${message}`),
    appendNotice: (message: string) => notices.push(message),
    appendCloudNotice: () => {},
    formatError: String,
    attachBinding: async () => {},
    transitionToNoSession: () => {},
    applyProviderSelection: async () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    applyModeSelection: async () => {},
    applyPermissionSelection: async () => {},
    currentExecutionMode: () => "build",
    currentPermissionLevel: () => "yolo",
    refreshWaitingRoomData: async () => {},
    remoteMachinesState: () => [],
    setRemoteMachinesState: () => {},
    reconcileWaitingRoom: () => {},
    setSlicesState: () => {},
    appLogger: null,
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    applySessionState: (nextSession: RuntimeSession) => { state = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    requestRootRender: () => {},
    scheduleTimer: () => 0,
    logViewDebug: () => {},
    describeRenderableDebug: () => "",
    currentFocusedRenderable: () => null,
    trackAgentFocusTransition: async (action: () => Promise<unknown>) => action(),
    setProviderRunState: () => {},
    resolveSessionAgent: () => ({ agent: null }),
    selectedWorkflowId: () => selectedWorkflowId,
    refreshSplitPaneFocusRepaint: () => {},
  } as any)

  const execute = (args: string[]) => handlers.handleWorkflowCommand({
    kind: "workflow",
    raw: `/workflow ${args.join(" ")}`,
    args,
  })

  await execute(["pause", "run-1"])
  await execute(["run-show", "run-1"])
  await execute(["run-get", "run-1"])
  await execute(["queue"])
  await execute(["queue", "create", "review", "2"])
  await execute(["queue", "rename", "default", "review"])
  await execute(["queue", "priority", "default", "4"])
  await execute(["queue", "enable", "default"])
  await execute(["queue", "disable", "default"])
  await execute(["queue", "edit", "prompt-1", "updated"])
  await execute(["queue", "move", "prompt-1", "review"])
  await execute(["queue", "remove", "prompt-1"])
  await execute(["queue", "clear", "default"])
  await execute(["queue", "delete", "default"])
  await execute(["endpoint", "rebind", "endpoint-1", "node-2"])
  await execute(["endpoint", "remove", "endpoint-1"])
  await execute(["delete"])

  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
    "PauseWorkflowRun",
    "GetWorkflowRun",
    "GetWorkflowRun",
    "ListWorkflowPromptQueues",
    "ListQueuedWorkflowPrompts",
    "CreateWorkflowPromptQueue",
    "UpdateWorkflowPromptQueue",
    "UpdateWorkflowPromptQueue",
    "UpdateWorkflowPromptQueue",
    "UpdateWorkflowPromptQueue",
    "UpdateQueuedWorkflowPrompt",
    "UpdateQueuedWorkflowPrompt",
    "RemoveQueuedWorkflowPrompt",
    "ClearWorkflowPromptQueue",
    "RemoveWorkflowPromptQueue",
    "ResolveWorkflow",
    "ApplyWorkflowDesignOp",
    "ResolveWorkflow",
    "ApplyWorkflowDesignOp",
    "ResolveWorkflow",
    "ApplyWorkflowDesignOp",
  ])
  assert.deepEqual(
    requests.filter((request) => request.UpdateWorkflowPromptQueue).map((request) => request.UpdateWorkflowPromptQueue.enabled),
    [null, null, true, false],
  )
  assert.equal(requests.some((request) => request.RemoveQueuedWorkflowPrompt), true)
  assert.equal(requests.some((request) => request.RemoveWorkflowPromptQueue), true)
  const designRequests = requests.filter((request) => request.ApplyWorkflowDesignOp)
  assert.deepEqual(designRequests.map((request) => request.ApplyWorkflowDesignOp.op), [
    { kind: "endpoint_update", workflow_id: "workflow-1", endpoint_id: "endpoint-1", patch: { entry_node_id: "node-2" } },
    { kind: "endpoint_remove", workflow_id: "workflow-1", endpoint_id: "endpoint-1" },
    { kind: "workflow_remove", workflow_id: "workflow-1" },
  ])
  assert.equal(designRequests.every((request) => request.ApplyWorkflowDesignOp.origin_client_id === "cli-1"), true)
  assert.equal(designRequests.every((request) => /^tui-/.test(request.ApplyWorkflowDesignOp.op_id)), true)
  assert.equal(notices.length, 2)
  assert.equal(footers.includes("info:paused workflow run run-1 [paused]"), true)
  assert.equal(footers.includes("info:deleted workflow workflow-1 (Review)"), true)
  assert.equal(selectedWorkflowId, null)
})

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "Review",
    nodes: [{ id: "node-1", agent_id: "agent-1" }, { id: "node-2", agent_id: "agent-2" }],
    edges: [],
    endpoints: [{ id: "endpoint-1", alias: "start", entry_node_id: "node-1" }],
    ...overrides,
  }
}

function workflowQueue(): WorkflowPromptQueueDefinition {
  return {
    id: "default",
    workflow_id: "workflow-1",
    alias: "default",
    priority: 0,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}

function workflowQueuedPrompt(): WorkflowQueuedPrompt {
  return {
    id: "prompt-1",
    queue_id: "default",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    prompt: "queued",
    source: "manual",
    status: "queued",
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}

function workflowRun(): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: "run",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 2,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

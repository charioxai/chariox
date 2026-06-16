import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import {
  handleWorkflowAddAllNodesCommand,
  handleWorkflowNodeCommand,
  type WorkflowNodeCommandContext,
  type WorkflowNodeCommandDeps,
} from "./workflow-node-command-handlers.js"

test("workflow node add and remove mutate the selected workflow", async () => {
  const harness = createHarness({
    agentsByRef: {
      planner: agent({ id: "agent-planner", alias: "planner" }),
    },
  })

  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "add", "workflow-1", "planner"])
  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "remove", "node-1"])

  assert.deepEqual(harness.calls, [
    "add:workflow-1:agent-planner",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:added workflow node node-1 for agent planner",
    "remove:workflow-1:node-1",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:removed workflow node node-1",
  ])
})

test("workflow node add can enable wait-for-all-inputs immediately", async () => {
  const harness = createHarness({
    agentsByRef: {
      joiner: agent({ id: "agent-joiner", alias: "joiner" }),
    },
  })

  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "add", "workflow-1", "joiner", "--wait-for-all-inputs"])

  assert.deepEqual(harness.calls, [
    "add:workflow-1:agent-joiner",
    "set-wait-inputs:workflow-1:node-1:true",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:added workflow node node-1 for agent joiner",
  ])
})


test("workflow add node all adds only missing session agents", async () => {
  const harness = createHarness({
    sessionAgents: [
      agent({ id: "agent-a", alias: "alpha" }),
      agent({ id: "agent-b", alias: "bravo" }),
      agent({ id: "agent-c", alias: "charlie" }),
    ],
    workflow: workflow({ nodes: [node({ id: "node-a", agent_id: "agent-a" })] }),
  })

  await handleWorkflowAddAllNodesCommand(harness.deps, harness.context, ["add", "node", "all"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "add:workflow-1:agent-b",
    "apply:session-1",
    "upsert:workflow-1",
    "add:workflow-1:agent-c",
    "apply:session-1",
    "upsert:workflow-1",
    "select:workflow-1",
    "footer:info:added 2 workflow nodes for bravo, charlie",
  ])
})

test("workflow add node all reports when all agents are already present", async () => {
  const harness = createHarness({
    sessionAgents: [agent({ id: "agent-a" })],
    workflow: workflow({ nodes: [node({ id: "node-a", agent_id: "agent-a" })] }),
  })

  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "add", "all"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "select:workflow-1",
    "footer:info:workflow workflow-1 already has nodes for all session agents",
  ])
})

test("workflow node runtime settings apply returned workflow state", async () => {
  const harness = createHarness()

  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "can-complete-run", "workflow-1", "node-1", "true"])
  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "can-emit-intermediate-output", "node-1", "false"])
  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "wait-for-all-inputs", "node-1", "true"])
  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "intermediate-output-schema", "node-1", "none"])
  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "max-turns", "workflow-1", "node-1", "2"])

  assert.deepEqual(harness.calls, [
    "set-complete:workflow-1:node-1:true",
    "apply:session-1",
    "upsert:workflow-1",
    "footer:info:workflow node node-1 can-complete-run set to true",
    "set-intermediate:workflow-1:node-1:false",
    "apply:session-1",
    "upsert:workflow-1",
    "footer:info:workflow node node-1 can-emit-intermediate-output set to false",
    "set-wait-inputs:workflow-1:node-1:true",
    "apply:session-1",
    "upsert:workflow-1",
    "footer:info:workflow node node-1 wait-for-all-inputs set to true",
    "set-schema:workflow-1:node-1:null",
    "apply:session-1",
    "upsert:workflow-1",
    "footer:info:workflow node node-1 intermediate-output-schema set to none",
    "set-max-turns:workflow-1:node-1:2",
    "apply:session-1",
    "upsert:workflow-1",
    "footer:info:workflow node node-1 max-turns set to 2",
  ])
})

test("workflow node command validates usage and unavailable runtime support", async () => {
  const missingSettings = createHarness({ runtimeSettings: false })
  await handleWorkflowNodeCommand(missingSettings.deps, missingSettings.context, ["node", "can-complete-run", "node-1", "true"])

  const invalidMaxTurns = createHarness()
  await handleWorkflowNodeCommand(invalidMaxTurns.deps, invalidMaxTurns.context, ["node", "max-turns", "node-1", "0"])

  const missingAgent = createHarness()
  await handleWorkflowNodeCommand(missingAgent.deps, missingAgent.context, ["node", "add", "missing"])

  assert.deepEqual(missingSettings.calls, [
    "footer:error:workflow runtime commands unavailable",
  ])
  assert.deepEqual(invalidMaxTurns.calls, [
    "footer:error:usage: /workflow node max-turns [workflow-ref] <node-id> <count|none>",
  ])
  assert.deepEqual(missingAgent.calls, [
    "footer:error:agent 'missing' not found",
  ])
})

test("workflow node extension commands explain collaborator-owned nodes without leaking hidden agent ids", async () => {
  const harness = createHarness({
    workflow: workflow({ nodes: [node({ id: "node-hidden", agent_id: "agent-hidden" })] }),
    sessionAgents: [agent({ id: "agent-a" })],
  })

  await handleWorkflowNodeCommand(harness.deps, harness.context, ["node", "extensions", "node-hidden"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "footer:error:Extensions are managed by the collaborator who owns this node.",
  ])
  assert.doesNotMatch(harness.calls.join("\n"), /agent-hidden/)
})

type HarnessOptions = Partial<WorkflowNodeCommandDeps> & {
  agentsByRef?: Record<string, AgentInstance>
  context?: Partial<WorkflowNodeCommandContext>
  runtimeSettings?: boolean
  selectedWorkflowRef?: string | null
  sessionAgents?: AgentInstance[]
  workflow?: WorkflowDefinition
}

function createHarness(options: HarnessOptions = {}) {
  const {
    agentsByRef = {},
    context: contextOverrides,
    runtimeSettings = true,
    selectedWorkflowRef = "workflow-1",
    sessionAgents = [agent({ id: "agent-a" })],
    workflow: currentWorkflow = workflow(),
    ...depOverrides
  } = options
  const calls: string[] = []
  const deps: WorkflowNodeCommandDeps = {
    currentWorkspaceTarget: () => "/tmp",
    resolveWorkflow: async (workflowRef) => {
      calls.push(`resolve:${workflowRef}`)
      return { workflow: { ...currentWorkflow, id: workflowRef } }
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    sessionState: () => session({ agents: sessionAgents }),
    resolveSessionAgent: (reference) => ({
      agent: reference ? agentsByRef[reference] ?? null : null,
      error: reference && agentsByRef[reference] ? null : `agent '${reference}' not found`,
    }),
    addWorkflowNode: async (workflowRef, agentId) => {
      calls.push(`add:${workflowRef}:${agentId}`)
      return { node: node({ agent_id: agentId }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    removeWorkflowNode: async (workflowRef, nodeId) => {
      calls.push(`remove:${workflowRef}:${nodeId}`)
      return { node: node({ id: nodeId }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    formatAgentLabel: (nextAgent) => nextAgent?.alias ?? nextAgent?.id ?? "agent",
    ...runtimeSettingDeps(calls, runtimeSettings),
    ...depOverrides,
  }
  const context: WorkflowNodeCommandContext = {
    firstWorkflowArgIsExplicit: (workflowRef) => Boolean(workflowRef && workflowRef.startsWith("workflow-")),
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function runtimeSettingDeps(calls: string[], enabled: boolean): Partial<WorkflowNodeCommandDeps> {
  if (!enabled) {
    return {}
  }
  return {
    setWorkflowNodeCanCompleteRun: async (workflowRef, nodeId, value) => {
      calls.push(`set-complete:${workflowRef}:${nodeId}:${String(value)}`)
      return { node: node({ id: nodeId, can_complete_workflow_run: value }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    setWorkflowNodeCanEmitIntermediateOutput: async (workflowRef, nodeId, value) => {
      calls.push(`set-intermediate:${workflowRef}:${nodeId}:${String(value)}`)
      return { node: node({ id: nodeId, can_emit_intermediate_run_output: value }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    setWorkflowNodeWaitForAllInputs: async (workflowRef, nodeId, value) => {
      calls.push(`set-wait-inputs:${workflowRef}:${nodeId}:${String(value)}`)
      return { node: node({ id: nodeId, wait_for_all_inputs: value }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    setWorkflowNodeIntermediateOutputSchema: async (workflowRef, nodeId, schemaRef) => {
      calls.push(`set-schema:${workflowRef}:${nodeId}:${schemaRef ?? "null"}`)
      return { node: node({ id: nodeId, intermediate_output_schema_ref: schemaRef }), workflow: workflow({ id: workflowRef }), session: session() }
    },
    setWorkflowNodeMaxTurns: async (workflowRef, nodeId, maxTurns) => {
      calls.push(`set-max-turns:${workflowRef}:${nodeId}:${maxTurns ?? "null"}`)
      return { node: node({ id: nodeId, max_turns: maxTurns }), workflow: workflow({ id: workflowRef }), session: session() }
    },
  }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-a",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: null,
    worktree_id: null,
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

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function node(overrides: Partial<WorkflowNodeDefinition> = {}): WorkflowNodeDefinition {
  return {
    id: "node-1",
    agent_id: "agent-a",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

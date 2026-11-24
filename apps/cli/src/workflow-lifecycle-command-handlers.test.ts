import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowRun,
} from "./cli-types.js"
import {
  handleWorkflowAliasCommand,
  handleWorkflowDeleteCommand,
  handleWorkflowFromAgentCommand,
  handleWorkflowListCommand,
  handleWorkflowNewCommand,
  handleWorkflowRootCommand,
  handleWorkflowShowCommand,
  type WorkflowLifecycleCommandContext,
  type WorkflowLifecycleCommandDeps,
} from "./workflow-lifecycle-command-handlers.js"

test("workflow root opens an existing session workflow", async () => {
  const harness = createHarness({
    sessionWorkflows: [workflow({ id: "workflow-session" })],
  })

  await handleWorkflowRootCommand(harness.deps)

  assert.deepEqual(harness.calls, [
    "select:workflow-session",
    "show",
  ])
})

test("workflow root creates the first workflow when the screen is already active", async () => {
  const harness = createHarness({ screenActive: true })

  await handleWorkflowRootCommand(harness.deps)

  assert.deepEqual(harness.calls, [
    "list",
    "create:null",
    "select:workflow-1",
    "apply:session-1",
    "footer:info:created workflow workflow-1",
  ])
})

test("workflow list formats workflow aliases", async () => {
  const harness = createHarness({
    listedWorkflows: [
      workflow({ id: "workflow-1", alias: "main" }),
      workflow({ id: "workflow-2" }),
    ],
  })

  await handleWorkflowListCommand(harness.deps)

  assert.deepEqual(harness.calls, [
    "list",
    "replace:workflow-1,workflow-2",
    "footer:info:workflows: workflow-1 (main), workflow-2",
  ])
})

test("workflow show and new project the selected workflow", async () => {
  const harness = createHarness({
    selectedWorkflowRef: "workflow-selected",
    resolvedWorkflow: workflow({ id: "workflow-selected", alias: "selected" }),
  })

  await handleWorkflowShowCommand(harness.deps, harness.context, ["show"])
  await handleWorkflowNewCommand(harness.deps, ["new", "fresh"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-selected",
    "upsert:workflow-selected",
    "select:workflow-selected",
    "show",
    "footer:info:workflow workflow-selected (selected)",
    "create:fresh",
    "select:workflow-1",
    "show",
    "apply:session-1",
    "footer:info:created workflow workflow-1 (fresh)",
  ])
})

test("opening a workflow loads active run details without overwriting a newer event", async () => {
  const definition = workflow()
  const run: WorkflowRun = {
    id: "run-1", workflow_id: definition.id, endpoint_id: "entry", entry_node_id: "node-1", status: "Running",
    invocation_prompt: "Review change", active_node_run_id: "node-run", node_runs: [], messages: [],
    created_at_ms: 1, started_at_ms: 2, completed_at_ms: null,
  }
  let current = session({ room_workflows: {
    session_id: "session-1", home_kernel_id: "home", revision: "active", workflow_count: 1, workflows: [{
      workflow_id: definition.id, workflow_revision: 1, label: "Review", state: "running", running_count: 1,
      paused_count: 0, queued_count: 0, endpoints: [], runs: [{ run_id: run.id, endpoint_id: "entry", status: "Running", can_pause: true, can_resume: false, can_stop: true }],
    }],
  } })
  let sends = 0
  const { deps, context } = createHarness({
    sessionState: () => current,
    upsertWorkflowDefinition: workflow => { current = { ...current, workflows: [workflow] } },
    getWorkflowRun: async id => {
      assert.equal(id, run.id)
      if (++sends === 2) current = { ...current, workflow_runs: [{ ...run, status: "Paused" }] }
      return { workflow_run: run }
    },
    applySessionState: state => { current = state },
  })

  await handleWorkflowShowCommand(deps, context, ["show", definition.id])
  assert.deepEqual(current.workflow_runs, [run])
  await handleWorkflowShowCommand(deps, context, ["show", definition.id])
  assert.equal(sends, 1)
  current = { ...current, workflow_runs: [] }
  await handleWorkflowShowCommand(deps, context, ["show", definition.id])
  assert.equal(current.workflow_runs?.[0]?.status, "Paused")
})

test("workflow alias validates missing and unknown workflows", async () => {
  const usage = createHarness()
  await handleWorkflowAliasCommand(usage.deps, ["workflow-1"])

  const unknown = createHarness({ aliasResult: null })
  await handleWorkflowAliasCommand(unknown.deps, ["workflow-missing", "main"])

  const success = createHarness()
  await handleWorkflowAliasCommand(success.deps, ["workflow-1", "main"])

  assert.equal(usage.calls[0]?.startsWith("footer:error:usage: /workflow | /workflow list"), true)
  assert.deepEqual(unknown.calls, [
    "alias:workflow-missing:main",
    "footer:error:unknown workflow: workflow-missing",
  ])
  assert.deepEqual(success.calls, [
    "alias:workflow-1:main",
    "upsert:workflow-1",
    "show",
    "footer:info:workflow workflow-1 aliased as main",
  ])
})

test("workflow delete removes the selected workflow and selects the next canvas", async () => {
  const nextSession = session({ workflows: [workflow({ id: "workflow-2" })] })
  const harness = createHarness({
    selectedWorkflowRef: "workflow-1",
    deleteWorkflow: async (workflowRef) => {
      harness.calls.push(`delete:${workflowRef}`)
      return { workflow: workflow({ id: workflowRef, alias: "main" }), session: nextSession }
    },
  })

  await handleWorkflowDeleteCommand(harness.deps, harness.context, ["delete"])

  assert.deepEqual(harness.calls, [
    "delete:workflow-1",
    "apply:session-1",
    "select:workflow-2",
    "footer:info:deleted workflow workflow-1 (main)",
  ])
})

type HarnessOptions = Partial<WorkflowLifecycleCommandDeps> & {
  aliasResult?: WorkflowDefinition | null
  context?: Partial<WorkflowLifecycleCommandContext>
  listedWorkflows?: WorkflowDefinition[]
  resolvedWorkflow?: WorkflowDefinition
  screenActive?: boolean
  selectedWorkflowRef?: string | null
  sessionWorkflows?: WorkflowDefinition[]
}

function createHarness(options: HarnessOptions = {}) {
  const {
    aliasResult = workflow({ alias: "main" }),
    context: contextOverrides,
    listedWorkflows = [],
    resolvedWorkflow = workflow(),
    screenActive = false,
    selectedWorkflowRef = "workflow-1",
    sessionWorkflows = [],
    ...depOverrides
  } = options
  const calls: string[] = []
  const deps: WorkflowLifecycleCommandDeps = {
    sessionState: () => session({ workflows: sessionWorkflows }),
    workflowScreenActive: () => screenActive,
    selectedWorkflowId: () => selectedWorkflowRef,
    showWorkflowScreen: () => {
      calls.push("show")
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    replaceWorkflowDefinitions: (workflows) => {
      calls.push(`replace:${workflows.map((entry) => entry.id).join(",")}`)
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    createWorkflow: async (alias) => {
      calls.push(`create:${alias ?? "null"}`)
      return { workflow: workflow({ alias: alias ?? null }), session: session() }
    },
    listWorkflows: async () => {
      calls.push("list")
      return listedWorkflows
    },
    resolveWorkflow: async (workflowRef) => {
      calls.push(`resolve:${workflowRef}`)
      return { workflow: { ...resolvedWorkflow, id: workflowRef } }
    },
    assignWorkflowAlias: async (workflowRef, alias) => {
      calls.push(`alias:${workflowRef}:${alias}`)
      return aliasResult ? { ...aliasResult, id: workflowRef, alias } : null
    },
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...depOverrides,
  }
  const context: WorkflowLifecycleCommandContext = {
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
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

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    connected_cli_count: 0,
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

test("workflow from-agent gives an agent its one-node trigger workflow", async () => {
  const harness = createHarness()
  const created: string[] = []
  const deps = {
    ...harness.deps,
    resolveSessionAgent: (reference?: string | null) => ({ agent: reference === "builder" ? { id: "agent-1" } : null }),
    createAgentWorkflow: async (agentId: string, reason: "trigger" | "deploy", alias?: string | null) => {
      created.push(`${agentId}:${reason}:${alias ?? "null"}`)
      return { workflow: workflow({ id: "workflow-7", alias: "builder-trigger" }), endpoint: { id: "endpoint-1" }, session: session() }
    },
  }

  await handleWorkflowFromAgentCommand(deps, ["from-agent", "builder", "trigger"])
  assert.deepEqual(created, ["agent-1:trigger:null"])
  assert.deepEqual(harness.calls, [
    "select:workflow-7",
    "show",
    "footer:info:created workflow workflow-7 (builder-trigger) for builder; next: /workflow trigger ... on endpoint endpoint-1",
  ])

  // A trigger class names the command that adds it.
  harness.calls.length = 0
  await handleWorkflowFromAgentCommand(deps, ["from-agent", "builder", "trigger", "schedule", "nightly"])
  await handleWorkflowFromAgentCommand(deps, ["from-agent", "builder", "trigger", "http"])
  assert.deepEqual(created.slice(1), ["agent-1:trigger:nightly", "agent-1:trigger:null"])
  assert.deepEqual(harness.calls.filter((call) => call.startsWith("footer")), [
    "footer:info:created workflow workflow-7 (builder-trigger) for builder; next: /workflow schedule add workflow-7 endpoint-1 --every 1h",
    "footer:info:created workflow workflow-7 (builder-trigger) for builder; next: /workflow trigger create workflow-7 endpoint-1 --route /",
  ])

  harness.calls.length = 0
  await handleWorkflowFromAgentCommand(deps, ["from-agent", "builder", "bind"])
  await handleWorkflowFromAgentCommand(deps, ["from-agent", "nobody", "deploy"])
  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow from-agent <agent-ref> trigger [http|schedule|notification]|deploy [alias]",
    "footer:error:unknown agent: nobody",
  ])
})

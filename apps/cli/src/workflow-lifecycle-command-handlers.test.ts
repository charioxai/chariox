import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
} from "./cli-types.js"
import {
  handleWorkflowAliasCommand,
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

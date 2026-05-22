import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowRun,
} from "./cli-types.js"
import {
  handleWorkflowInvokeCommand,
  type WorkflowInvokeCommandContext,
  type WorkflowInvokeCommandDeps,
} from "./workflow-invoke-command-handler.js"

test("workflow invoke command starts a workflow run with an explicit workflow ref", async () => {
  const harness = createHarness({
    context: {
      firstWorkflowArgIsExplicit: () => true,
      workflowRefOrSelected: (workflowRef) => workflowRef ?? null,
    },
    invokeWorkflowEndpoint: async (workflowRef, endpointRef, prompt) => {
      harness.calls.push(`invoke:${workflowRef}:${endpointRef}:${prompt ?? "null"}`)
      return {
        workflow: workflow({ id: workflowRef }),
        endpoint: endpoint({ id: endpointRef }),
        session: session({ id: "session-1" }),
        workflow_run: run({ id: "run-1", status: "RUNNING" }),
      }
    },
  })

  await handleWorkflowInvokeCommand(harness.deps, harness.context, ["run", "workflow-1", "start", "ship", "it"])

  assert.deepEqual(harness.calls, [
    "invoke:workflow-1:start:ship it",
    "apply:session-1",
    "upsert:workflow-1",
    "select:workflow-1",
    "show",
    "footer:info:started workflow run run-1 [running]",
  ])
})

test("workflow invoke command queues launch when the runtime reports an active run", async () => {
  const harness = createHarness({
    context: {
      firstWorkflowArgIsExplicit: () => false,
      workflowRefOrSelected: () => "selected-workflow",
    },
    invokeWorkflowEndpoint: async (workflowRef, endpointRef, prompt) => {
      harness.calls.push(`invoke:${workflowRef}:${endpointRef}:${prompt ?? "null"}`)
      return {
        workflow: workflow({ id: workflowRef }),
        endpoint: endpoint({ id: endpointRef }),
        session: session({ id: "session-2" }),
        queued_prompt: {
          id: "queue-1",
          queue_id: "default",
          workflow_id: workflowRef,
          endpoint_id: endpointRef,
          prompt: prompt ?? null,
          source: "manual",
          status: "queued",
          created_at_ms: 1,
          updated_at_ms: 1,
        },
      }
    },
  })

  await handleWorkflowInvokeCommand(harness.deps, harness.context, ["start", "endpoint-1", "later"])

  assert.deepEqual(harness.calls, [
    "invoke:selected-workflow:endpoint-1:later",
    "apply:session-2",
    "upsert:selected-workflow",
    "select:selected-workflow",
    "show",
    "footer:info:queued workflow prompt queue-1",
  ])
})

test("workflow invoke command validates target and runtime availability", async () => {
  const missingTarget = createHarness({
    context: {
      firstWorkflowArgIsExplicit: () => false,
      workflowRefOrSelected: () => null,
    },
  })
  await handleWorkflowInvokeCommand(missingTarget.deps, missingTarget.context, ["run"])

  const unavailableRuntime = createHarness({ invokeWorkflowEndpoint: undefined })
  await handleWorkflowInvokeCommand(unavailableRuntime.deps, unavailableRuntime.context, ["run", "endpoint-1"])

  assert.deepEqual(missingTarget.calls, [
    "footer:error:usage: /workflow run|start [workflow-ref] <endpoint-ref> [prompt]",
  ])
  assert.deepEqual(unavailableRuntime.calls, [
    "footer:error:workflow runtime commands unavailable",
  ])
})

type HarnessOptions = Omit<Partial<WorkflowInvokeCommandDeps>, "invokeWorkflowEndpoint"> & {
  invokeWorkflowEndpoint?: WorkflowInvokeCommandDeps["invokeWorkflowEndpoint"] | undefined
  context?: Partial<WorkflowInvokeCommandContext>
}

function createHarness(overrides: HarnessOptions) {
  const { context: contextOverrides, ...depOverrides } = overrides
  const { invokeWorkflowEndpoint, ...plainDepOverrides } = depOverrides
  const calls: string[] = []
  const deps: WorkflowInvokeCommandDeps = {
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    showWorkflowScreen: () => {
      calls.push("show")
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...plainDepOverrides,
  }
  if (invokeWorkflowEndpoint !== undefined || !("invokeWorkflowEndpoint" in depOverrides)) {
    deps.invokeWorkflowEndpoint = invokeWorkflowEndpoint ?? (async (workflowRef, endpointRef, prompt) => ({
      workflow: workflow({ id: workflowRef }),
      endpoint: endpoint({ id: endpointRef }),
      session: session({ id: "session-1" }),
      workflow_run: run({ id: prompt ?? "run-1", status: "RUNNING" }),
    }))
  }
  const context: WorkflowInvokeCommandContext = {
    firstWorkflowArgIsExplicit: () => false,
    workflowRefOrSelected: (workflowRef) => workflowRef ?? "workflow-1",
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

function endpoint(overrides: Partial<WorkflowEndpointDefinition> = {}): WorkflowEndpointDefinition {
  return {
    id: "endpoint-1",
    alias: null,
    entry_node_id: "node-1",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_root: "/tmp/workspace",
    agents: [],
    workflows: [],
    ...overrides,
  } as RuntimeSession
}

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    status: "RUNNING",
    started_at_ms: 1,
    completed_at_ms: null,
    failure_count: 0,
    ...overrides,
  } as WorkflowRun
}

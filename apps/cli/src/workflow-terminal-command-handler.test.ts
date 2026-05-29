import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition } from "./cli-types.js"
import {
  handleWorkflowTerminalCommand,
  type WorkflowTerminalCommandDeps,
} from "./workflow-terminal-command-handler.js"

test("workflow terminal command opens the resolved workflow terminal", async () => {
  const harness = createHarness({
    workflowRefOrSelected: () => "workflow-ref",
    resolveWorkflow: async (workflowRef) => {
      harness.calls.push(`resolve:${workflowRef}`)
      return { workflow: workflow({ id: "workflow-1" }) }
    },
  })

  await handleWorkflowTerminalCommand(harness.deps, ["terminal"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-ref",
    "upsert:workflow-1",
    "select:workflow-1",
    "show",
    "open:workflow-1:logs",
    "footer:info:opened workflow logs pane for workflow-1",
  ])
})

test("workflow terminal command falls back to the first session workflow", async () => {
  const harness = createHarness({
    sessionWorkflows: () => [workflow({ id: "workflow-session" })],
    workflowRefOrSelected: () => null,
    resolveWorkflow: async (workflowRef) => {
      harness.calls.push(`resolve:${workflowRef}`)
      return { workflow: workflow({ id: workflowRef }) }
    },
  })

  await handleWorkflowTerminalCommand(harness.deps, ["terminal"])

  assert.equal(harness.calls[0], "resolve:workflow-session")
})

test("workflow terminal command reports usage without a workflow target", async () => {
  const harness = createHarness({
    workflowRefOrSelected: () => null,
    sessionWorkflows: () => [],
  })

  await handleWorkflowTerminalCommand(harness.deps, ["terminal"])

  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow terminal [workflow-ref]",
  ])
})

test("workflow pane command opens requested detail mode", async () => {
  const harness = createHarness({
    workflowRefOrSelected: (workflowRef) => workflowRef ?? "workflow-selected",
  })

  await handleWorkflowTerminalCommand(harness.deps, ["pane", "trace", "workflow-1"])

  assert.deepEqual(harness.calls, [
    "upsert:workflow-1",
    "select:workflow-1",
    "show",
    "open:workflow-1:trace",
    "footer:info:opened workflow trace pane for workflow-1",
  ])
})

function createHarness(overrides: Partial<WorkflowTerminalCommandDeps>) {
  const calls: string[] = []
  const deps: WorkflowTerminalCommandDeps = {
    sessionWorkflows: () => [],
    workflowRefOrSelected: (workflowRef) => workflowRef ?? null,
    resolveWorkflow: async (workflowRef) => ({ workflow: workflow({ id: workflowRef }) }),
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    showWorkflowScreen: () => {
      calls.push("show")
    },
    openWorkflowTerminalPanel: (workflowId, mode) => {
      calls.push(`open:${workflowId}:${mode}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...overrides,
  }
  return { calls, deps }
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

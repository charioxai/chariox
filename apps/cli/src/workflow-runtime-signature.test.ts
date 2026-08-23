import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowEndpointRuntimeInstance,
  WorkflowRun,
} from "./cli-types.js"
import { workflowRuntimeSignature } from "./workflow-runtime-signature.js"

test("workflow runtime signature tracks run and instance status changes", () => {
  const base = session()
  const changedRun = session({
    workflow_runs: [run({ id: "run-1", status: "Completed" })],
  })
  const changedInstance = session({
    workflow_runtime_instances: [instance({ status: "idle", active_run_id: null })],
  })
  const unchanged = session()

  assert.notEqual(workflowRuntimeSignature(base), workflowRuntimeSignature(changedRun))
  assert.notEqual(workflowRuntimeSignature(base), workflowRuntimeSignature(changedInstance))
  assert.equal(workflowRuntimeSignature(base), workflowRuntimeSignature(unchanged))
})

test("workflow runtime signature tracks endpoint capacity changes", () => {
  const base = session({ workflows: [workflow(2)] })
  const changed = session({ workflows: [workflow(4)] })

  assert.notEqual(workflowRuntimeSignature(base), workflowRuntimeSignature(changed))
})

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workflow_runs: [run()],
    workflow_runtime_instances: [instance()],
    ...overrides,
  } as RuntimeSession
}

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: null,
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 10,
    started_at_ms: null,
    completed_at_ms: null,
    ...overrides,
  }
}

function instance(overrides: Partial<WorkflowEndpointRuntimeInstance> = {}): WorkflowEndpointRuntimeInstance {
  return {
    id: "instance-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    workflow_revision: 1,
    ordinal: 1,
    primary: true,
    node_agent_ids: {},
    worktree_id: "worktree-1",
    status: "busy",
    active_run_id: "run-1",
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}

function workflow(maxInstances: number): NonNullable<RuntimeSession["workflows"]>[number] {
  return {
    id: "workflow-1",
    alias: "reviewer",
    revision: 1,
    max_concurrent: 4,
    flush_agent_context_before_run: true,
    nodes: [],
    edges: [],
    endpoints: [{
      id: "endpoint-1",
      alias: "review",
      entry_node_id: "node-1",
      owner_user_id: "local",
      max_instances: maxInstances,
    }],
  }
}

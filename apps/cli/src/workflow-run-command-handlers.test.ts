import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowRun } from "./cli-types.js"
import {
  formatWorkflowRunSummary,
  handleWorkflowRunCancelCommand,
  handleWorkflowRunPauseCommand,
  handleWorkflowRunResumeCommand,
  handleWorkflowRunShowCommand,
  handleWorkflowRunsCommand,
  type WorkflowRunCommandDeps,
} from "./workflow-run-command-handlers.js"

test("workflow runs command lists run summaries for an optional workflow", async () => {
  const calls: string[] = []
  const deps = createDeps(calls, {
    listWorkflowRuns: async (workflowRef) => {
      calls.push(`list:${workflowRef}`)
      return [
        workflowRun({ id: "run-1", status: "Running" }),
        workflowRun({
          id: "run-2",
          status: "Failed",
          failure_events: [{
            kind: "Validation",
            source_node_run_id: "node-run-1",
            edge_ids: [],
            message: "bad output",
            timestamp_ms: 1,
          }],
        }),
      ]
    },
  })

  await handleWorkflowRunsCommand(deps, ["runs", "workflow-1"])

  assert.deepEqual(calls, [
    "list:workflow-1",
    "footer:info:workflow runs: run-1 [running], run-2 [failed, failures 1]",
  ])
})

test("workflow runs command reports empty and unavailable runtime states", async () => {
  const emptyCalls: string[] = []
  await handleWorkflowRunsCommand(createDeps(emptyCalls, {
    listWorkflowRuns: async () => [],
  }), ["runs"])

  const unavailableCalls: string[] = []
  await handleWorkflowRunsCommand(createDeps(unavailableCalls), ["runs"])

  assert.deepEqual(emptyCalls, ["footer:info:no workflow runs in session"])
  assert.deepEqual(unavailableCalls, ["footer:error:workflow runtime commands unavailable"])
})

test("workflow cancel, pause, and resume commands apply returned sessions", async () => {
  const calls: string[] = []
  const deps = createDeps(calls, {
    cancelWorkflowRun: async (runRef) => ({
      workflow_run: workflowRun({ id: runRef, status: "Stopped" }),
      session: runtimeSession({ id: "session-cancelled" }),
    }),
    pauseWorkflowRun: async (runRef) => ({
      workflow_run: workflowRun({ id: runRef, status: "Paused" }),
      session: runtimeSession({ id: "session-paused" }),
    }),
    resumeWorkflowRun: async (runRef) => ({
      workflow_run: workflowRun({ id: runRef, status: "Running" }),
      session: runtimeSession({ id: "session-resumed" }),
    }),
  })

  await handleWorkflowRunCancelCommand(deps, ["cancel", "run-1"])
  await handleWorkflowRunPauseCommand(deps, ["pause", "run-1"])
  await handleWorkflowRunResumeCommand(deps, ["resume", "run-1"])

  assert.deepEqual(calls, [
    "session:session-cancelled",
    "footer:info:cancelled workflow run run-1 [stopped]",
    "session:session-paused",
    "footer:info:paused workflow run run-1 [paused]",
    "session:session-resumed",
    "footer:info:resumed workflow run run-1 [running]",
  ])
})

test("workflow run-show and run-get render the full run payload", async () => {
  const calls: string[] = []
  const deps = createDeps(calls, {
    appendNotice: (message) => calls.push(`notice:${message}`),
    getWorkflowRun: async (runRef) => ({
      workflow_run: workflowRun({ id: runRef, status: "Completed" }),
    }),
  })

  await handleWorkflowRunShowCommand(deps, ["run-show", "run-1"])
  await handleWorkflowRunShowCommand(deps, ["run-get", "run-2"])

  assert.match(calls[0] ?? "", /notice:\{[\s\S]*"id": "run-1"/)
  assert.equal(calls[1], "footer:info:workflow run run-1 [completed]")
  assert.match(calls[2] ?? "", /notice:\{[\s\S]*"id": "run-2"/)
  assert.equal(calls[3], "footer:info:workflow run run-2 [completed]")
})

test("workflow cancel, pause, and resume commands validate usage and runtime support", async () => {
  const calls: string[] = []
  const deps = createDeps(calls)

  await handleWorkflowRunCancelCommand(deps, ["cancel"])
  await handleWorkflowRunCancelCommand(deps, ["cancel", "run-1"])
  await handleWorkflowRunPauseCommand(deps, ["pause"])
  await handleWorkflowRunPauseCommand(deps, ["pause", "run-1"])
  await handleWorkflowRunResumeCommand(deps, ["resume"])
  await handleWorkflowRunResumeCommand(deps, ["resume", "run-1"])

  assert.deepEqual(calls, [
    "footer:error:usage: /workflow cancel <run-ref>",
    "footer:error:workflow runtime commands unavailable",
    "footer:error:usage: /workflow pause <run-ref>",
    "footer:error:workflow runtime commands unavailable",
    "footer:error:usage: /workflow resume <run-ref>",
    "footer:error:workflow runtime commands unavailable",
  ])
})

test("workflow run-show validates usage and runtime support", async () => {
  const calls: string[] = []
  const deps = createDeps(calls)

  await handleWorkflowRunShowCommand(deps, ["run-show"])
  await handleWorkflowRunShowCommand(deps, ["run-get", "run-1"])

  assert.deepEqual(calls, [
    "footer:error:usage: /workflow run-show <run-ref>",
    "footer:error:workflow runtime commands unavailable",
  ])
})

test("formatWorkflowRunSummary includes failure count only when present", () => {
  assert.equal(formatWorkflowRunSummary(workflowRun({ status: "Completed" })), "run-1 [completed]")
  assert.equal(
    formatWorkflowRunSummary(workflowRun({
      status: "Failed",
      failure_events: [
        { kind: "Validation", source_node_run_id: "node-run-1", edge_ids: [], message: "one", timestamp_ms: 1 },
        { kind: "Runtime", source_node_run_id: "node-run-2", edge_ids: [], message: "two", timestamp_ms: 2 },
      ],
    })),
    "run-1 [failed, failures 2]",
  )
})

function createDeps(
  calls: string[],
  overrides: Partial<WorkflowRunCommandDeps> = {},
): WorkflowRunCommandDeps {
  return {
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    applySessionState: (session) => {
      calls.push(`session:${session.id}`)
    },
    ...overrides,
  }
}

function workflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
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
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
    ...overrides,
  }
}

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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
    max_agents: 0,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

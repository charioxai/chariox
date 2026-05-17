import assert from "node:assert/strict"
import test from "node:test"

import type { QueuedWorkflowLaunch, RuntimeSession } from "./cli-types.js"
import {
  formatQueuedWorkflowLaunch,
  handleWorkflowQueueCommand,
  type WorkflowQueueCommandDeps,
} from "./workflow-queue-command-handlers.js"

test("workflow queue command lists queued launches", async () => {
  const harness = createHarness({
    listQueuedWorkflowLaunches: async () => [
      queuedLaunch({
        id: "queue-1",
        invocation_prompt: "x".repeat(60),
        watchdog_id: "watchdog-1",
      }),
    ],
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "list"])

  assert.deepEqual(harness.calls, [
    'footer:info:workflow queue: queue-1 [manual] workflow=workflow-1 endpoint=endpoint-1 watchdog=watchdog-1 queued_at=10 prompt="xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx..."',
  ])
})

test("workflow queue command flushes and applies the returned session", async () => {
  const session = runtimeSession({ id: "session-next" })
  const harness = createHarness({
    clearQueuedWorkflowLaunches: async () => ({
      queued_launches: [queuedLaunch({ id: "queue-1" }), queuedLaunch({ id: "queue-2" })],
      session,
    }),
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "flush"])

  assert.deepEqual(harness.calls, [
    "session:session-next",
    "footer:info:cleared 2 queued workflow launches",
  ])
})

test("workflow queue command removes a queued launch and reports missing runtime support", async () => {
  const session = runtimeSession({ id: "session-next" })
  const harness = createHarness({
    removeQueuedWorkflowLaunch: async (queueItemRef) => ({
      queued_launch: queuedLaunch({ id: queueItemRef }),
      session,
    }),
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "remove", "queue-1"])
  await handleWorkflowQueueCommand(createHarness({}).deps, ["queue", "remove", "queue-1"])

  assert.deepEqual(harness.calls, [
    "session:session-next",
    "footer:info:removed queued workflow launch queue-1",
  ])
})

test("workflow queue command validates action usage", async () => {
  const harness = createHarness({})

  await handleWorkflowQueueCommand(harness.deps, ["queue", "remove"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "unknown"])

  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow queue remove <queue-item-ref>",
    "footer:error:usage: /workflow queue [list|flush|remove <queue-item-ref>]",
  ])
})

test("formatQueuedWorkflowLaunch omits empty optional fields", () => {
  assert.equal(
    formatQueuedWorkflowLaunch(queuedLaunch({ source: "watchdog", invocation_prompt: "", watchdog_id: null })),
    "queue-1 [watchdog] workflow=workflow-1 endpoint=endpoint-1 queued_at=10",
  )
})

function createHarness(overrides: Partial<WorkflowQueueCommandDeps>) {
  const calls: string[] = []
  const deps: WorkflowQueueCommandDeps = {
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    applySessionState: (session) => {
      calls.push(`session:${session.id}`)
    },
    ...overrides,
  }
  return { calls, deps }
}

function queuedLaunch(overrides: Partial<QueuedWorkflowLaunch> = {}): QueuedWorkflowLaunch {
  return {
    id: "queue-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    invocation_prompt: null,
    source: "manual",
    queued_at_ms: 10,
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

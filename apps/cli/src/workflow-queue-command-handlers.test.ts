import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowPromptQueueDefinition, WorkflowQueuedPrompt } from "./cli-types.js"
import {
  formatWorkflowQueuedPrompt,
  handleWorkflowQueueCommand,
  type WorkflowQueueCommandDeps,
} from "./workflow-queue-command-handlers.js"

test("workflow queue command lists queues and queued prompts", async () => {
  const harness = createHarness({
    listWorkflowPromptQueues: async () => [queue()],
    listQueuedWorkflowPrompts: async () => [queuedPrompt({ prompt: "x".repeat(60) })],
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "list"])

  assert.match(harness.calls.at(-1) ?? "", /workflow queues: default\(default\) priority=0 depth=1/)
  assert.match(harness.calls.at(-1) ?? "", /prompts: prompt-1/)
})

test("workflow queue command clears and applies the returned session", async () => {
  const session = runtimeSession({ id: "session-next" })
  const harness = createHarness({
    clearWorkflowPromptQueue: async () => ({
      queued_prompts: [queuedPrompt({ id: "prompt-1" }), queuedPrompt({ id: "prompt-2" })],
      session,
    }),
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "clear"])

  assert.deepEqual(harness.calls, [
    "session:session-next",
    "footer:info:cleared 2 queued workflow prompts from default",
  ])
})

test("workflow queue command removes a queued prompt and reports missing runtime support", async () => {
  const session = runtimeSession({ id: "session-next" })
  const harness = createHarness({
    removeQueuedWorkflowPrompt: async (queueItemRef: string) => ({
      queued_prompt: queuedPrompt({ id: queueItemRef }),
      session,
    }),
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "remove", "prompt-1"])
  await handleWorkflowQueueCommand(createHarness({}).deps, ["queue", "remove", "prompt-1"])

  assert.deepEqual(harness.calls, [
    "session:session-next",
    "footer:info:removed queued workflow prompt prompt-1",
  ])
})

test("workflow queue command validates action usage", async () => {
  const harness = createHarness({})

  await handleWorkflowQueueCommand(harness.deps, ["queue", "remove"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "unknown"])

  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow queue remove <queue-item-ref>",
    "footer:error:usage: /workflow queue [list|create|rename|priority|edit|move|remove|clear]",
  ])
})

test("formatWorkflowQueuedPrompt omits empty optional fields", () => {
  assert.equal(
    formatWorkflowQueuedPrompt(queuedPrompt({ source: "watchdog", prompt: "", watchdog_id: null })),
    "prompt-1 [watchdog] queue=default endpoint=endpoint-1 status=queued",
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

function queue(overrides: Partial<WorkflowPromptQueueDefinition> = {}): WorkflowPromptQueueDefinition {
  return {
    id: "default",
    alias: "default",
    priority: 0,
    enabled: true,
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}

function queuedPrompt(overrides: Partial<WorkflowQueuedPrompt> = {}): WorkflowQueuedPrompt {
  return {
    id: "prompt-1",
    queue_id: "default",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    prompt: null,
    source: "manual",
    status: "queued",
    created_at_ms: 10,
    updated_at_ms: 10,
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

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
    listQueuedWorkflowPrompts: async () => [
      queuedPrompt({ prompt: "x".repeat(60) }),
      queuedPrompt({ id: "prompt-other", workflow_id: "workflow-2" }),
    ],
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "list"])

  assert.match(harness.calls.at(-1) ?? "", /workflow queues: default\(default\) priority=0 depth=1/)
  assert.match(harness.calls.at(-1) ?? "", /prompts: prompt-1/)
  assert.doesNotMatch(harness.calls.at(-1) ?? "", /prompt-other/)
})

test("workflow queue command accepts explicit workflow refs", async () => {
  const calls: string[] = []
  const harness = createHarness({
    selectedWorkflowId: () => "workflow-selected",
    listWorkflowPromptQueues: async (workflowRef) => {
      calls.push(`list:${workflowRef}`)
      return [queue({ workflow_id: workflowRef ?? "missing" })]
    },
    listQueuedWorkflowPrompts: async () => [queuedPrompt({ workflow_id: "workflow-explicit" })],
    createWorkflowPromptQueue: async (workflowRef, alias, priority) => {
      calls.push(`create:${workflowRef}:${alias}:${priority}`)
      return { queue: queue({ workflow_id: workflowRef ?? "missing", alias, priority }), session: runtimeSession() }
    },
    clearWorkflowPromptQueue: async (workflowRef, queueRef) => {
      calls.push(`clear:${workflowRef}:${queueRef}`)
      return { queued_prompts: [], session: runtimeSession() }
    },
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "--workflow", "workflow-explicit", "list"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "create", "--workflow", "workflow-explicit", "urgent", "9"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "clear", "--workflow", "workflow-explicit", "default"])

  assert.deepEqual(calls, [
    "list:workflow-explicit",
    "create:workflow-explicit:urgent:9",
    "clear:workflow-explicit:default",
  ])
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

test("workflow queue command enables, disables, and deletes queue definitions", async () => {
  const harness = createHarness({
    selectedWorkflowId: () => "workflow-1",
    updateWorkflowPromptQueue: async (workflowRef, queueRef, patch) => {
      harness.calls.push(`update:${workflowRef}:${queueRef}:${JSON.stringify(patch)}`)
      return {
        queue: queue({ id: queueRef, enabled: patch.enabled ?? true }),
        session: runtimeSession({ id: `session-${patch.enabled ? "enabled" : "disabled"}` }),
      }
    },
    removeWorkflowPromptQueue: async (workflowRef, queueRef) => {
      harness.calls.push(`delete:${workflowRef}:${queueRef}`)
      return { queue: queue({ id: queueRef }), session: runtimeSession({ id: "session-deleted" }) }
    },
  })

  await handleWorkflowQueueCommand(harness.deps, ["queue", "enable", "review"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "disable", "review"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "delete", "review"])

  assert.deepEqual(harness.calls, [
    "update:workflow-1:review:{\"enabled\":true}",
    "session:session-enabled",
    "footer:info:enabled workflow queue review(default) priority=0",
    "update:workflow-1:review:{\"enabled\":false}",
    "session:session-disabled",
    "footer:info:disabled workflow queue review(default) priority=0 disabled",
    "delete:workflow-1:review",
    "session:session-deleted",
    "footer:info:deleted workflow queue review(default) priority=0",
  ])
})

test("workflow queue command validates action usage", async () => {
  const harness = createHarness({})

  await handleWorkflowQueueCommand(harness.deps, ["queue", "remove"])
  await handleWorkflowQueueCommand(harness.deps, ["queue", "unknown"])

  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow queue remove [--workflow <workflow-ref>] <queue-item-ref>",
    "footer:error:usage: /workflow queue [list|create|rename|priority|enable|disable|delete|edit|move|remove|clear|flush]",
  ])
})

test("formatWorkflowQueuedPrompt omits empty optional fields", () => {
  assert.equal(
    formatWorkflowQueuedPrompt(queuedPrompt({ source: "watchdog", prompt: "", watchdog_id: null })),
    "prompt-1 [watchdog] workflow=workflow-1 queue=default endpoint=endpoint-1 status=queued",
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
    workflow_id: "workflow-1",
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
    project_id: "project-default",
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

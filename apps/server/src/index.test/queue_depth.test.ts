import type { LocalIpcClient } from "@chariox/kernel-client/ipc"
import type { WorkflowQueuedPrompt } from "@chariox/kernel-client/kernel-types"

import {
  assert,
  baseConfig,
  test,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"
import {
  isPendingPublicationQueuedPrompt,
  lookupPublicationQueueDepth,
  publicationQueueDepth,
} from "../publication-status.js"

const defaultQueueId = "queue-default-id"
const priorityQueueId = "queue-priority-id"
const publication: WorkflowPublicationConfig = {
  ...baseConfig,
  workflow_ref: "workflow-1",
  endpoint_ref: "endpoint-1",
  queue_ref: "default",
}

function queuedPrompt(overrides: Partial<WorkflowQueuedPrompt>): WorkflowQueuedPrompt {
  return {
    id: overrides.id ?? "prompt-1",
    queue_id: overrides.queue_id ?? defaultQueueId,
    workflow_id: overrides.workflow_id ?? "workflow-1",
    endpoint_id: overrides.endpoint_id ?? "endpoint-1",
    source: overrides.source ?? "manual",
    status: overrides.status ?? "queued",
    created_at_ms: overrides.created_at_ms ?? 0,
    updated_at_ms: overrides.updated_at_ms ?? 0,
    ...overrides,
  }
}

function queueList(queues: Array<{ id: string; alias: string }>) {
  return {
    WorkflowPromptQueuesListed: {
      queues: queues.map((queue) => ({
        ...queue,
        workflow_id: "workflow-1",
        priority: 0,
        enabled: true,
        created_at_ms: 0,
        updated_at_ms: 0,
      })),
    },
  }
}

function fakeClient(responses: Record<string, unknown>[]): Pick<LocalIpcClient, "send"> {
  return {
    send: async () => responses.shift() ?? {},
  } as unknown as Pick<LocalIpcClient, "send">
}

test("queue depth counts only queued or dispatching prompts for this workflow/endpoint/queue", () => {
  const prompts: WorkflowQueuedPrompt[] = [
    queuedPrompt({ id: "a", status: "queued" }),
    queuedPrompt({ id: "b", status: "dispatching" }),
    queuedPrompt({ id: "c", status: "running" }),
    queuedPrompt({ id: "d", status: "completed" }),
    queuedPrompt({ id: "e", status: "cancelled" }),
  ]
  assert.equal(publicationQueueDepth(prompts, publication, defaultQueueId), 2)
})

test("queue depth excludes prompts for other workflows, endpoints, and queues", () => {
  const prompts: WorkflowQueuedPrompt[] = [
    queuedPrompt({ id: "match" }),
    queuedPrompt({ id: "other-workflow", workflow_id: "workflow-2" }),
    queuedPrompt({ id: "other-endpoint", endpoint_id: "endpoint-2" }),
    queuedPrompt({ id: "other-queue", queue_id: priorityQueueId }),
  ]
  assert.equal(publicationQueueDepth(prompts, publication, defaultQueueId), 1)
})

test("queue depth matches an ingress prompt by its publication queue ref", () => {
  const prompt = queuedPrompt({
    id: "pub",
    queue_id: "queue-uuid-123",
    publication_invocation: {
      publication_id: "pub-test",
      invocation_id: "inv-1",
      transport: "human_http",
      endpoint_id: "endpoint-1",
      queue_ref: "default",
    },
  })
  assert.equal(isPendingPublicationQueuedPrompt(prompt, publication, defaultQueueId), true)
  assert.equal(publicationQueueDepth([prompt], publication, defaultQueueId), 1)
})

test("queue depth counts a scheduled prompt by its concrete queue id", () => {
  const prompt = queuedPrompt({
    id: "scheduled",
    source: "scheduled",
    queue_id: defaultQueueId,
  })

  assert.equal(publicationQueueDepth([prompt], publication, defaultQueueId), 1)
})

test("queue depth respects a non-default configured queue", () => {
  const scoped: WorkflowPublicationConfig = { ...publication, queue_ref: "priority" }
  const prompts: WorkflowQueuedPrompt[] = [
    queuedPrompt({ id: "default-queue", queue_id: defaultQueueId }),
    queuedPrompt({ id: "priority-queue", queue_id: priorityQueueId }),
  ]
  assert.equal(publicationQueueDepth(prompts, scoped, priorityQueueId), 1)
})

test("lookupPublicationQueueDepth resolves the configured queue before counting prompts", async () => {
  const client = fakeClient([{
    QueuedWorkflowPromptsListed: {
      queued_prompts: [
        queuedPrompt({ id: "a", status: "queued" }),
        queuedPrompt({ id: "b", status: "dispatching" }),
        queuedPrompt({ id: "c", status: "completed" }),
      ],
    },
  }, queueList([{ id: defaultQueueId, alias: "default" }])])

  assert.equal(await lookupPublicationQueueDepth(client, publication), 2)
})

test("lookupPublicationQueueDepth is fail-safe and returns null on lookup error", async () => {
  const client = {
    send: async () => { throw new Error("kernel unavailable") },
  } as unknown as Pick<LocalIpcClient, "send">

  assert.equal(await lookupPublicationQueueDepth(client, publication), null)
})

test("lookupPublicationQueueDepth returns zero when the prompt response is empty", async () => {
  const client = fakeClient([{}, queueList([{ id: defaultQueueId, alias: "default" }])])

  assert.equal(await lookupPublicationQueueDepth(client, publication), 0)
})

test("lookupPublicationQueueDepth returns null when the configured queue cannot resolve", async () => {
  const client = fakeClient([{ QueuedWorkflowPromptsListed: { queued_prompts: [] } }, queueList([])])

  assert.equal(await lookupPublicationQueueDepth(client, publication), null)
})

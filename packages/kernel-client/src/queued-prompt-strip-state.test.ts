import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./kernel-types.js"
import {
  queuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry,
  syncQueuedPromptTranscriptEntriesByAgent,
  syncQueuedPromptTranscriptEntriesForAgent,
  type QueuedPromptStripSourceEntry,
} from "./queued-prompt-strip-state.js"

test("queuedPromptStripItemsForAgent projects queued prompts for the agent", () => {
  const items = queuedPromptStripItemsForAgent(session(), [], "agent-1")

  assert.deepEqual(items.map((item) => ({
    promptId: item.promptId,
    agentId: item.agentId,
    sourceAttachmentId: item.sourceAttachmentId,
    prompt: item.prompt,
    status: item.status,
    attachmentCount: item.attachmentCount,
    canSteer: item.canSteer,
    canCancel: item.canCancel,
  })), [{
    promptId: "prompt-1",
    agentId: "agent-1",
    sourceAttachmentId: "attachment-1",
    prompt: "queued prompt",
    status: "queued",
    attachmentCount: 2,
    canSteer: true,
    canCancel: true,
  }])
})

test("queuedPromptStripItemsForAgent overlays optimistic transcript action state", () => {
  const entries: QueuedPromptStripSourceEntry[] = [{
    id: 1,
    role: "user",
    text: "queued prompt",
    queuedPrompt: {
      promptId: "prompt-1",
      agentId: "agent-1",
      status: "steering",
      attachmentCount: 0,
      steerDisabled: false,
      canSteer: false,
      canCancel: false,
      steerDisabledReason: "This prompt is no longer waiting in the queue.",
      cancelDisabledReason: "This prompt is no longer waiting in the queue.",
    },
  }]

  const [item] = queuedPromptStripItemsForAgent(session(), entries, "agent-1")

  assert.equal(item?.status, "steering")
  assert.equal(item?.canSteer, false)
  assert.equal(item?.canCancel, false)
  assert.equal(item?.attachmentCount, 2)
})

test("queuedPromptStripItemsForAgent overlays external status overrides", () => {
  const [item] = queuedPromptStripItemsForAgent(session(), [], "agent-1", [{
    promptId: "prompt-1",
    agentId: "agent-1",
    status: "cancelling",
    steerDisabled: true,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: "This prompt is currently being cancelled.",
    cancelDisabledReason: "This prompt is currently being cancelled.",
  }])

  assert.deepEqual({
    status: item?.status,
    canSteer: item?.canSteer,
    canCancel: item?.canCancel,
    steerDisabledReason: item?.steerDisabledReason,
    cancelDisabledReason: item?.cancelDisabledReason,
    prompt: item?.prompt,
    attachmentCount: item?.attachmentCount,
  }, {
    status: "cancelling",
    canSteer: false,
    canCancel: false,
    steerDisabledReason: "This prompt is currently being cancelled.",
    cancelDisabledReason: "This prompt is currently being cancelled.",
    prompt: "queued prompt",
    attachmentCount: 2,
  })
})

test("queuedPromptStripItemsForAgent uses projected queue controls and pending prompt id", () => {
  const [item] = queuedPromptStripItemsForAgent(session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "draft-queued",
          pending_prompt_id: "pending-prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued prompt\n",
          attachments: [],
          status: "queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        queued_prompt_controls: {
          "pending-prompt-1": {
            prompt_id: "pending-prompt-1",
            status: "dispatching",
            can_steer: false,
            can_cancel: false,
            steer_disabled_reason: "This prompt is no longer waiting in the queue.",
            cancel_disabled_reason: "This prompt is no longer waiting in the queue.",
          },
        },
      },
    },
  }), [], "agent-1")

  assert.deepEqual({
    promptId: item?.promptId,
    status: item?.status,
    canSteer: item?.canSteer,
    canCancel: item?.canCancel,
  }, {
    promptId: "pending-prompt-1",
    status: "dispatching",
    canSteer: false,
    canCancel: false,
  })
})

test("queuedPromptStripItemsForAgent preserves transcript rows when projection is unavailable", () => {
  const entries: QueuedPromptStripSourceEntry[] = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: {
      promptId: "prompt-preserved",
      agentId: "agent-1",
      status: "queued",
      attachmentCount: 1,
      steerDisabled: false,
      canSteer: true,
      canCancel: true,
      steerDisabledReason: null,
      cancelDisabledReason: null,
    },
  }]
  const items = queuedPromptStripItemsForAgent({
    ...session(),
    prompt_states: undefined,
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  } as unknown as RuntimeSession, entries, "agent-1")

  assert.deepEqual(items.map((item) => item.promptId), ["prompt-preserved"])
})

test("queuedPromptStripItemsForAgent applies status overrides when preserving transcript rows", () => {
  const entries: QueuedPromptStripSourceEntry[] = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: {
      promptId: "prompt-preserved",
      agentId: "agent-1",
      status: "queued",
      attachmentCount: 1,
      steerDisabled: false,
      canSteer: true,
      canCancel: true,
      steerDisabledReason: null,
      cancelDisabledReason: null,
    },
  }]
  const [item] = queuedPromptStripItemsForAgent({
    ...session(),
    prompt_states: undefined,
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  } as unknown as RuntimeSession, entries, "agent-1", [{
    promptId: "prompt-preserved",
    agentId: "agent-1",
    status: "steering",
    steerDisabled: true,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: "This prompt is currently being steered.",
    cancelDisabledReason: "This prompt is currently being steered.",
  }])

  assert.equal(item?.status, "steering")
  assert.equal(item?.canSteer, false)
  assert.equal(item?.canCancel, false)
})

test("queuedPromptStripItemToTranscriptEntry adapts strip actions to transcript action path", () => {
  const [item] = queuedPromptStripItemsForAgent(session(), [], "agent-1")
  const entry = queuedPromptStripItemToTranscriptEntry(item!)

  assert.equal(entry.role, "user")
  assert.equal(entry.text, "queued prompt")
  assert.deepEqual(entry.queuedPrompt, {
    promptId: "prompt-1",
    agentId: "agent-1",
    status: "queued",
    attachmentCount: 2,
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("syncQueuedPromptTranscriptEntriesForAgent removes stale queued rows when projection is authoritative", () => {
  const existing = [
    { id: 7, role: "assistant", text: "ready" },
    {
      id: 8,
      role: "user",
      text: "legacy queued",
      queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
    },
    { id: 9, role: "assistant", text: "still here" },
  ]

  const synced = syncQueuedPromptTranscriptEntriesForAgent(existing, session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued prompt",
          status: "queued",
        }],
      },
    },
  }), "agent-1")

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries, [
    { id: 1, role: "assistant", text: "ready" },
    { id: 2, role: "assistant", text: "still here" },
  ])
})

test("syncQueuedPromptTranscriptEntriesForAgent preserves legacy queued rows when projection is unavailable", () => {
  const existing = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-preserved"),
  }]

  const synced = syncQueuedPromptTranscriptEntriesForAgent(existing, session({
    prompt_states: undefined,
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  }), "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, existing)
})

test("syncQueuedPromptTranscriptEntriesByAgent reports changed agent ids", () => {
  const synced = syncQueuedPromptTranscriptEntriesByAgent({
    "agent-1": [{
      id: 1,
      role: "user",
      text: "legacy queued",
      queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
    }],
  }, session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  }))

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.changedAgentIds, ["agent-1"])
  assert.deepEqual(synced.entriesByAgent["agent-1"], [])
})

function queuedPrompt(agentId: string, promptId: string): NonNullable<QueuedPromptStripSourceEntry["queuedPrompt"]> {
  return {
    promptId,
    agentId,
    status: "queued",
    attachmentCount: 0,
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  }
}

function session(overrides: Record<string, unknown> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: "session",
    agents: [{ id: "agent-1" }],
    queued_prompts: [{
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "queued prompt\n",
      attachments: [{ kind: "file" }, { kind: "image" }],
      status: "queued",
    }],
    ...overrides,
  } as unknown as RuntimeSession
}

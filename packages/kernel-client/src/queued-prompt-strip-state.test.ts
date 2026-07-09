import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./kernel-types.js"
import {
  queuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry,
  syncQueuedPromptTranscriptEntriesByAgent,
  syncQueuedPromptTranscriptEntriesByAgentWithPreviews,
  syncQueuedPromptTranscriptEntriesForAgent,
  type QueuedPromptStripSourceEntry,
  type QueuedPromptTranscriptPreviewEntry,
} from "./queued-prompt-strip-state.js"

test("queuedPromptStripItemsForAgent projects queued prompts for the agent", () => {
  const items = queuedPromptStripItemsForAgent(session(), [], "agent-1")

  assert.deepEqual(items.map((item) => ({
    promptId: item.promptId,
    agentId: item.agentId,
    sourceAttachmentId: item.sourceAttachmentId,
    prompt: item.prompt,
    promptOrigin: item.promptOrigin,
    status: item.status,
    attachmentCount: item.attachmentCount,
    canSteer: item.canSteer,
    canCancel: item.canCancel,
  })), [{
    promptId: "prompt-1",
    agentId: "agent-1",
    sourceAttachmentId: "attachment-1",
    prompt: "queued prompt",
    promptOrigin: null,
    status: "queued",
    attachmentCount: 2,
    canSteer: true,
    canCancel: true,
  }])
})

test("queuedPromptStripItemsForAgent replaces optimistic transcript action state from projection", () => {
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

  assert.equal(item?.status, "queued")
  assert.equal(item?.canSteer, true)
  assert.equal(item?.canCancel, true)
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
          pending_prompt_id: "external:codex:thread-1:turn-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued prompt\n",
          attachments: [],
          status: "queued",
          prompt_origin: " External ",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        queued_prompt_controls: {
          "external:codex:thread-1:turn-1": {
            prompt_id: "external:codex:thread-1:turn-1",
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
    promptOrigin: item?.promptOrigin,
    externalProvider: item?.externalProvider,
    externalProviderSessionId: item?.externalProviderSessionId,
    externalProviderTurnId: item?.externalProviderTurnId,
    status: item?.status,
    canSteer: item?.canSteer,
    canCancel: item?.canCancel,
  }, {
    promptId: "external:codex:thread-1:turn-1",
    promptOrigin: "external",
    externalProvider: undefined,
    externalProviderSessionId: undefined,
    externalProviderTurnId: undefined,
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
    promptOrigin: "external",
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
  assert.equal(items[0]?.promptOrigin, "external")
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
  assert.equal(entry.promptId, "prompt-1")
  assert.equal(entry.promptOrigin, null)
  assert.deepEqual(entry.queuedPrompt, {
    promptId: "prompt-1",
    agentId: "agent-1",
    promptOrigin: null,
    status: "queued",
    attachmentCount: 2,
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("queuedPromptStripItemToTranscriptEntry mirrors prompt ownership at transcript level", () => {
  const [item] = queuedPromptStripItemsForAgent(session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-external",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "external queued prompt",
          status: "queued",
          prompt_origin: "external",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
        }],
      },
    },
  }), [], "agent-1")

  const entry = queuedPromptStripItemToTranscriptEntry(item!)

  assert.equal(entry.promptOrigin, "external")
  assert.equal(entry.promptId, "prompt-external")
  assert.equal(entry.externalProvider, "codex")
  assert.equal(entry.externalProviderSessionId, "thread-1")
  assert.equal(entry.externalProviderTurnId, "user-1")
  assert.equal(entry.queuedPrompt?.promptOrigin, "external")
  assert.equal(entry.queuedPrompt?.promptId, "prompt-external")
  assert.equal(entry.queuedPrompt?.externalProvider, "codex")
  assert.equal(entry.queuedPrompt?.externalProviderSessionId, "thread-1")
  assert.equal(entry.queuedPrompt?.externalProviderTurnId, "user-1")
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

test("syncQueuedPromptTranscriptEntriesForAgent leaves transcript unchanged when queued projection is live", () => {
  const synced = syncQueuedPromptTranscriptEntriesForAgent([
    { id: 1, role: "assistant", text: "ready" },
  ], session(), "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, [
    { id: 1, role: "assistant", text: "ready" },
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

test("syncQueuedPromptTranscriptEntriesByAgent ignores projection keys outside session agents", () => {
  const synced = syncQueuedPromptTranscriptEntriesByAgent({
    "agent-1": [{
      id: 1,
      role: "assistant",
      text: "ready",
    }],
  }, session({
    prompt_states: {
      "agent-ghost": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-ghost": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  }))

  assert.equal(synced.changed, false)
  assert.deepEqual(Object.keys(synced.entriesByAgent), ["agent-1"])
})

test("syncQueuedPromptTranscriptEntriesByAgentWithPreviews refreshes changed agent previews", () => {
  const entriesByAgent: Record<string, QueuedPromptTranscriptPreviewEntry[]> = {
    "agent-1": [{
      id: 1,
      role: "user",
      text: "legacy queued",
      queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
    }],
  }
  const synced = syncQueuedPromptTranscriptEntriesByAgentWithPreviews(entriesByAgent, session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  }))

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entriesByAgent["agent-1"], [])
  assert.deepEqual(synced.previews, { "agent-1": "" })
})

test("syncQueuedPromptTranscriptEntriesByAgentWithPreviews preserves panes without authoritative projection", () => {
  const existing: QueuedPromptTranscriptPreviewEntry[] = [{
    id: 1,
    role: "user",
    text: "preserved queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-preserved"),
  }]
  const synced = syncQueuedPromptTranscriptEntriesByAgentWithPreviews({ "agent-1": existing }, session({
    prompt_states: undefined,
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
      },
    },
  }))

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entriesByAgent["agent-1"], existing)
  assert.deepEqual(synced.previews, {})
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
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued prompt\n",
          attachments: [{ kind: "file" }, { kind: "image" }],
          status: "queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        queued_prompt_count: 1,
        queued_prompt_controls: {
          "prompt-1": {
            prompt_id: "prompt-1",
            status: "queued",
            can_steer: true,
            can_cancel: true,
            steer_disabled_reason: null,
            cancel_disabled_reason: null,
          },
        },
      },
    },
    ...overrides,
  } as unknown as RuntimeSession
}

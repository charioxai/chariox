import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  queuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry,
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
  const entries: TranscriptEntry[] = [{
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

test("queuedPromptStripItemsForAgent preserves transcript rows when projection is unavailable", () => {
  const entries: TranscriptEntry[] = [{
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

function session(): RuntimeSession {
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
  } as unknown as RuntimeSession
}

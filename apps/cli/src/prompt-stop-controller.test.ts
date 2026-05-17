import assert from "node:assert/strict"
import test from "node:test"

import { createPromptStopController } from "./prompt-stop-controller.js"

test("request cancels the active prompt once and stays in flight until reset", async () => {
  let cancellationCount = 0
  let requestedAgentId: string | null = null
  const controller = createPromptStopController({
    getAttachment: () => ({ id: "attachment-1" }),
    getActivePrompt: () => ({ target_agent_id: "agent-1" }),
    getSessionId: () => "session-1",
    getFallbackStreamingAgentId: () => null,
    cancelActivePrompt: async (sessionId, attachmentId) => {
      assert.equal(sessionId, "session-1")
      assert.equal(attachmentId, "attachment-1")
      cancellationCount += 1
    },
    onCancellationRequested: (targetAgentId) => {
      requestedAgentId = targetAgentId
    },
    onCancellationFailed: () => {},
  })

  assert.equal(await controller.request(), true)
  assert.equal(await controller.request(), false)
  assert.equal(cancellationCount, 1)
  assert.equal(requestedAgentId, "agent-1")
  assert.equal(controller.isInFlight(), true)

  controller.reset()
  assert.equal(controller.isInFlight(), false)
})

test("request is idle without an active prompt or attachment", async () => {
  let cancellations = 0
  let attachment: { id: string } | null = null
  let activePrompt: { target_agent_id?: string | null } | null = {
    target_agent_id: "agent-1",
  }
  const controller = createPromptStopController({
    getAttachment: () => attachment,
    getActivePrompt: () => activePrompt,
    getSessionId: () => "session-1",
    getFallbackStreamingAgentId: () => null,
    cancelActivePrompt: async () => {
      cancellations += 1
    },
    onCancellationRequested: () => {},
    onCancellationFailed: () => {},
  })

  assert.equal(await controller.request(), false)
  attachment = { id: "attachment-1" }
  activePrompt = null
  assert.equal(await controller.request(), false)
  assert.equal(cancellations, 0)
})

test("request clears in-flight state and reports cancellation failures", async () => {
  let failure: unknown
  const controller = createPromptStopController({
    getAttachment: () => ({ id: "attachment-1" }),
    getActivePrompt: () => ({ target_agent_id: "agent-1" }),
    getSessionId: () => "session-1",
    getFallbackStreamingAgentId: () => null,
    cancelActivePrompt: async () => {
      throw new Error("cancel failed")
    },
    onCancellationRequested: () => {},
    onCancellationFailed: (error) => {
      failure = error
    },
  })

  assert.equal(await controller.request(), false)

  assert.match(failure instanceof Error ? failure.message : String(failure), /cancel failed/)
  assert.equal(controller.isInFlight(), false)
})

test("request falls back to the current streaming agent when active prompt target is missing", async () => {
  let requestedAgentId: string | null = null
  const controller = createPromptStopController({
    getAttachment: () => ({ id: "attachment-1" }),
    getActivePrompt: () => ({}),
    getSessionId: () => "session-1",
    getFallbackStreamingAgentId: () => "streaming-agent",
    cancelActivePrompt: async () => {},
    onCancellationRequested: (targetAgentId) => {
      requestedAgentId = targetAgentId
    },
    onCancellationFailed: () => {},
  })

  assert.equal(await controller.request(), true)

  assert.equal(requestedAgentId, "streaming-agent")
})

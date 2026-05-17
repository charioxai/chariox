import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import {
  createFocusedInteractionChoiceController,
  type FocusedInteractionChoiceControllerDeps,
} from "./focused-interaction-choice-controller.js"

test("focused interaction choice submit ignores unavailable state", async () => {
  const harness = createHarness({ interaction: null })

  assert.equal(await harness.controller.submitChoice(), false)
  assert.deepEqual(harness.responses(), [])
})

test("focused interaction choice submit answers selected choices", async () => {
  const harness = createHarness()
  harness.selectedIndexes.set("interaction-1", 1)

  assert.equal(await harness.controller.submitChoice(), true)

  assert.deepEqual(harness.responses(), [{
    sessionId: "session-1",
    interactionId: "interaction-1",
    choiceId: "deny",
    customReply: null,
  }])
  assert.equal(harness.appliedSessions().at(-1)?.id, "session-answered")
  assert.deepEqual(harness.customReplyDeletes(), ["interaction-1"])
  assert.equal(harness.customEditingValues().at(-1)?.editing, false)
  assert.equal(harness.footerMessages().at(-1)?.message, "interaction answered")
})

test("focused interaction choice submit enters custom editing for incomplete custom replies", async () => {
  const harness = createHarness()
  harness.selectedIndexes.set("interaction-1", 2)

  assert.equal(await harness.controller.submitChoice(), true)

  assert.deepEqual(harness.responses(), [])
  assert.equal(harness.customEditingValues().at(-1)?.editing, true)
  assert.equal(harness.renderCount(), 1)
  assert.equal(harness.layoutCount(), 1)
})

test("focused interaction choice submit reports response failures", async () => {
  const harness = createHarness({
    respondToInteraction: async () => {
      throw new Error("denied")
    },
  })

  assert.equal(await harness.controller.submitChoice(0), true)

  assert.equal(harness.footerMessages().at(-1)?.message, "denied")
  assert.equal(harness.footerMessages().at(-1)?.tone, "error")
})

test("focused interaction choice cycle updates selection and exits custom editing", () => {
  const harness = createHarness()
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customEditing.add("interaction-1")

  assert.equal(harness.controller.cycleChoice(1), true)

  assert.equal(harness.selectedIndexes.get("interaction-1"), 0)
  assert.equal(harness.customEditing.has("interaction-1"), false)
  assert.equal(harness.renderCount(), 1)
  assert.equal(harness.layoutCount(), 1)
})

test("focused interaction choice key handling edits custom replies", () => {
  const harness = createHarness()
  harness.customEditing.add("interaction-1")
  harness.customReplies.set("interaction-1", "o")
  const events: string[] = []

  const handled = harness.controller.handleKey({
    name: "k",
    preventDefault: () => events.push("prevent"),
    stopPropagation: () => events.push("stop"),
  })

  assert.equal(handled, true)
  assert.deepEqual(events, ["prevent", "stop"])
  assert.equal(harness.customReplies.get("interaction-1"), "ok")
  assert.equal(harness.renderCount(), 1)
  assert.equal(harness.layoutCount(), 1)
})

function createHarness(options: {
  interaction?: RuntimeInteraction | null
  attached?: boolean
  respondToInteraction?: FocusedInteractionChoiceControllerDeps["respondToInteraction"]
} = {}) {
  const selectedIndexes = new Map<string, number>()
  const customReplies = new Map<string, string>()
  const customEditing = new Set<string>()
  const responses: Array<{
    sessionId: string
    interactionId: string
    choiceId: string
    customReply: string | null
  }> = []
  const appliedSessions: RuntimeSession[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const customReplyDeletes: string[] = []
  const customEditingValues: Array<{ interactionId: string; editing: boolean }> = []
  let renderCount = 0
  let layoutCount = 0

  const controller = createFocusedInteractionChoiceController({
    getFocusedInteraction: () => options.interaction === undefined ? interactionFixture() : options.interaction,
    isAttached: () => options.attached ?? true,
    getSessionId: () => "session-1",
    getSelectedIndex: (interactionId) => selectedIndexes.get(interactionId),
    setSelectedIndex: (interactionId, index) => {
      selectedIndexes.set(interactionId, index)
    },
    getCustomReply: (interactionId) => customReplies.get(interactionId) ?? "",
    setCustomReply: (interactionId, reply) => {
      customReplies.set(interactionId, reply)
    },
    clearCustomReply: (interactionId) => {
      customReplyDeletes.push(interactionId)
      customReplies.delete(interactionId)
    },
    isCustomEditing: (interactionId) => customEditing.has(interactionId),
    setCustomEditing: (interactionId, editing) => {
      customEditingValues.push({ interactionId, editing })
      if (editing) {
        customEditing.add(interactionId)
      } else {
        customEditing.delete(interactionId)
      }
    },
    renderAgentInteractions: () => {
      renderCount += 1
    },
    applyResponseLayout: () => {
      layoutCount += 1
    },
    respondToInteraction: async (sessionId, interactionId, choiceId, customReply) => {
      responses.push({ sessionId, interactionId, choiceId, customReply })
      return options.respondToInteraction
        ? options.respondToInteraction(sessionId, interactionId, choiceId, customReply)
        : runtimeSession("session-answered")
    },
    applySessionState: (session) => {
      appliedSessions.push(session)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    selectedIndexes,
    customReplies,
    customEditing,
    responses: () => responses,
    appliedSessions: () => appliedSessions,
    footerMessages: () => footerMessages,
    customReplyDeletes: () => customReplyDeletes,
    customEditingValues: () => customEditingValues,
    renderCount: () => renderCount,
    layoutCount: () => layoutCount,
  }
}

function interactionFixture(): RuntimeInteraction {
  return {
    id: "interaction-1",
    agent_id: "agent-1",
    kind: "choice",
    level: "info",
    message: "Approve?",
    choices: [
      { id: "allow", label: "Allow", reply: "allow" },
      { id: "deny", label: "Deny", reply: "deny" },
    ],
    custom_choice: {
      id: "custom",
      label: "Custom",
      min_length: 2,
      max_length: 10,
    },
    requested_at_ms: 1,
  }
}

function runtimeSession(id: string): RuntimeSession {
  return {
    id,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 1,
    agents: [],
    config_state: {
      version: 1,
      values: {},
    },
  }
}

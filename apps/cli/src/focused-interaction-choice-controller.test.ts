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

test("focused interaction choice clears secret custom replies on submit", async () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret" }) })
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customReplies.set("interaction-1", "secret-value")
  harness.customEditing.add("interaction-1")

  assert.equal(await harness.controller.submitChoice(), true)

  assert.deepEqual(harness.responses(), [{
    sessionId: "session-1",
    interactionId: "interaction-1",
    choiceId: "custom",
    customReply: "secret-value",
  }])
  assert.equal(harness.customReplies.has("interaction-1"), false)
  assert.deepEqual(harness.customReplyDeletes(), ["interaction-1"])
  assert.deepEqual(harness.customEditingValues().filter((value) => !value.editing), [{
    interactionId: "interaction-1",
    editing: false,
  }])
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

test("focused interaction secret replies keep the typed case and are submitted exactly", async () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret", maxLength: 64 }) })
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customEditing.add("interaction-1")

  for (const event of [
    { name: "p", shift: true, sequence: "P" },
    { name: "a", sequence: "a" },
    { name: "1", shift: true, sequence: "!" },
    { name: "space", sequence: " " },
    { name: "ü", sequence: "ü" },
    { name: "x", shift: true, sequence: "\u001b[120;2u" },
  ]) {
    assert.equal(harness.controller.handleKey(event), true, event.name)
  }
  assert.equal(harness.customReplies.get("interaction-1"), "Pa! üX")

  assert.equal(await harness.controller.submitChoice(), true)
  assert.equal(harness.responses().at(-1)?.customReply, "Pa! üX")
})

test("focused interaction paste goes into the custom reply, never the prompt", () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret", maxLength: 64 }) })
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customEditing.add("interaction-1")
  harness.customReplies.set("interaction-1", "Ab")
  const events: string[] = []

  const handled = harness.controller.handlePaste({
    text: "Cd!@# Ünïcode\u{1F511}\n",
    preventDefault: () => events.push("prevent"),
    stopPropagation: () => events.push("stop"),
  })

  assert.equal(handled, true)
  assert.deepEqual(events, ["prevent", "stop"])
  assert.equal(harness.customReplies.get("interaction-1"), "AbCd!@# Ünïcode\u{1F511}")
  assert.equal(harness.renderCount(), 1)
})

test("focused interaction paste starts the reply on a selected secret choice", () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret", maxLength: 64 }) })
  harness.selectedIndexes.set("interaction-1", 2)

  assert.equal(harness.controller.handlePaste({ text: "MixedCase" }), true)

  assert.equal(harness.customEditing.has("interaction-1"), true)
  assert.equal(harness.customReplies.get("interaction-1"), "MixedCase")
})

test("focused interaction paste refuses multi-line text without passing it on", () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret", maxLength: 64 }) })
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customEditing.add("interaction-1")
  const events: string[] = []

  assert.equal(harness.controller.handlePaste({
    text: "first\nsecond",
    preventDefault: () => events.push("prevent"),
  }), true)

  assert.deepEqual(events, ["prevent"])
  assert.equal(harness.customReplies.has("interaction-1"), false)
  assert.equal(harness.footerMessages().at(-1)?.tone, "error")
})

test("focused interaction paste refuses a paste whose raw text held ANSI codes", () => {
  const harness = createHarness({ interaction: interactionFixture({ customInputKind: "secret", maxLength: 64 }) })
  harness.selectedIndexes.set("interaction-1", 2)
  harness.customEditing.add("interaction-1")

  // OpenTUI strips the escape sequence from `text`; the raw paste keeps it.
  assert.equal(harness.controller.handlePaste({ text: "AbCd", rawText: "Ab\u001b[31mCd" }), true)

  assert.equal(harness.customReplies.has("interaction-1"), false)
  assert.equal(harness.footerMessages().at(-1)?.tone, "error")
})

test("focused interaction paste leaves other pastes to the prompt", () => {
  const harness = createHarness()
  assert.equal(harness.controller.handlePaste({ text: "prompt text" }), false)
  harness.selectedIndexes.set("interaction-1", 2)
  assert.equal(harness.controller.handlePaste({ text: "not secret, not editing" }), false)
  harness.customEditing.add("interaction-1")
  assert.equal(harness.controller.handlePaste({ text: "taken", defaultPrevented: true }), false)
  assert.equal(harness.customReplies.has("interaction-1"), false)
})

test("MP-08/MP-11 a provider login code reports progress, never only 'interaction answered'", async () => {
  const actions: string[] = []
  const login = loginInteractionFixture()
  const harness = createHarness({ interaction: login, providerLogin: loginActions(actions) })
  harness.selectedIndexes.set(login.id, 1)
  harness.customEditing.add(login.id)
  harness.customReplies.set(login.id, "code#state")

  assert.equal(await harness.controller.submitChoice(), true)

  assert.deepEqual(harness.responses(), [{ sessionId: "session-1", interactionId: login.id, choiceId: "provider-response", customReply: "code#state" }])
  assert.deepEqual(actions, ["codeSent"])
  assert.equal(harness.customReplies.has(login.id), false)
  assert.ok(!harness.footerMessages().some((value) => value.message === "interaction answered"))
})

test("MP-08/MP-11 O and C act on the login link only while the focused code field is empty", async () => {
  const actions: string[] = []
  const login = loginInteractionFixture()
  const harness = createHarness({ interaction: login, providerLogin: loginActions(actions) })
  harness.selectedIndexes.set(login.id, 1)
  harness.customEditing.add(login.id)

  assert.equal(harness.controller.handleKey({ name: "o", sequence: "o" }), true)
  assert.equal(harness.controller.handleKey({ name: "c", sequence: "c" }), true)
  assert.deepEqual(actions, ["openLink", "copyLink"])
  assert.equal(harness.customReplies.get(login.id) ?? "", "")

  harness.customReplies.set(login.id, "x")
  assert.equal(harness.controller.handleKey({ name: "o", sequence: "o" }), true)
  assert.equal(harness.customReplies.get(login.id), "xo")
  assert.deepEqual(actions, ["openLink", "copyLink"])

  // Not editing: letters belong to the prompt.
  harness.customEditing.delete(login.id)
  harness.customReplies.delete(login.id)
  assert.equal(harness.controller.handleKey({ name: "c", sequence: "c" }), false)
  assert.deepEqual(actions, ["openLink", "copyLink"])
})

function loginActions(actions: string[]): NonNullable<FocusedInteractionChoiceControllerDeps["providerLogin"]> {
  return {
    stripState: () => ({ view: { url: "https://claude.ai/oauth/authorize" } }),
    codeSent: () => { actions.push("codeSent"); return true },
    openLink: async () => { actions.push("openLink") },
    copyLink: async () => { actions.push("copyLink") },
  }
}

function loginInteractionFixture(): RuntimeInteraction {
  return {
    id: "provider-auth-recovery:login-1", agent_id: "agent-1", kind: "choice", level: "warning", message: "Sign in",
    choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }],
    custom_choice: { id: "provider-response", label: "Send response", input_kind: "secret", min_length: 1, max_length: 8192 },
    requested_at_ms: 1,
    provider_login: { kernel_id: "fleet", login: { provider: "claude", account_profile: "p", login_kind: "terminal_setup_token", login_id: "login-1", auth_url: "https://claude.ai/oauth/authorize" }, terminal_output_base64: "" },
  }
}

function createHarness(options: {
  interaction?: RuntimeInteraction | null
  attached?: boolean
  respondToInteraction?: FocusedInteractionChoiceControllerDeps["respondToInteraction"]
  providerLogin?: FocusedInteractionChoiceControllerDeps["providerLogin"]
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
    ...(options.providerLogin ? { providerLogin: options.providerLogin } : {}),
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

function interactionFixture(options: {
  customInputKind?: "text" | "secret" | null
  maxLength?: number
} = {}): RuntimeInteraction {
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
      max_length: options.maxLength ?? 10,
      ...(options.customInputKind !== undefined ? { input_kind: options.customInputKind } : {}),
    },
    requested_at_ms: 1,
  }
}

function runtimeSession(id: string): RuntimeSession {
  return {
    id,
    project_id: "project-default",
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

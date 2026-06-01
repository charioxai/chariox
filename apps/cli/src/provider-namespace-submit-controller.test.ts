import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import {
  createProviderNamespaceSubmitController,
  type ProviderNamespaceSubmitControllerDeps,
} from "./provider-namespace-submit-controller.js"
import type { PromptSubmissionResult } from "./prompt-runtime-api.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"

test("provider namespace submit ignores non-provider namespace prompts", async () => {
  const harness = createHarness()

  assert.equal(await harness.controller.submit("hello"), false)
  assert.deepEqual(harness.submissions(), [])
})

test("provider namespace submit reports unavailable focused providers", async () => {
  const harness = createHarness({ focusedProvider: "codex" })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.equal(harness.footerMessages().at(-1)?.message, "/opencode is unavailable while the focused agent uses codex")
  assert.deepEqual(harness.submissions(), [])
})

test("provider namespace submit requires an attachment", async () => {
  const harness = createHarness({ attachment: null })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.equal(harness.footerMessages().at(-1)?.message, "No session attached.")
  assert.equal(harness.clearPromptCount(), 1)
  assert.deepEqual(harness.submissions(), [])
})

test("provider namespace submit forwards commands and applies submission state", async () => {
  const harness = createHarness()

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "/session list\n",
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "/opencode session list\n", agentId: "agent-1" }])
  assert.equal(harness.appliedSessions().at(-1)?.id, "session-submitted")
  assert.deepEqual(harness.streamingAgentIds(), ["agent-submitted"])
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "/opencode session list" }])
  assert.equal(harness.commandCenterClearCount(), 1)
  assert.equal(harness.workingValues().at(-1), true)
})

test("provider namespace submit drops stale focused agent ids", async () => {
  const harness = createHarness({
    focusedAgentId: "old-agent",
    hasAgent: (agentId) => agentId === "agent-1",
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: null,
    prompt: "/session list\n",
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "/opencode session list\n", agentId: null }])
})

test("provider namespace submit restores UI after submission failure", async () => {
  const harness = createHarness({
    submitProviderNamespacePrompt: async () => {
      throw new Error("provider failed")
    },
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.equal(harness.logErrors().at(-1)?.message, "provider namespace command failed")
  assert.equal(harness.restoredSnapshots().at(-1)?.rawPrompt, "/opencode session list")
  assert.deepEqual(harness.clearedBusyAgents(), ["agent-busy"])
  assert.deepEqual(harness.submittingAgentIds(), [null])
  assert.deepEqual(harness.submittingValues(), [false])
  assert.equal(harness.workingValues().at(-1), false)
  assert.equal(harness.fatalErrors().at(-1), "provider failed")
})

function createHarness(options: {
  focusedProvider?: "opencode" | "codex" | "claude" | null
  focusedAgentId?: string | null
  hasAgent?: (agentId: string) => boolean
  attachment?: RuntimeAttachment | null
  submitProviderNamespacePrompt?: ProviderNamespaceSubmitControllerDeps["submitProviderNamespacePrompt"]
} = {}) {
  const submissions: Array<{ attachmentId: string; targetAgentId: string | null; prompt: string }> = []
  const appendedPrompts: Array<{ text: string; agentId: string | null | undefined }> = []
  const appliedSessions: RuntimeSession[] = []
  const streamingAgentIds: Array<string | null> = []
  const recordedHistory: Array<{ sessionId: string; rawPrompt: string }> = []
  const restoredSnapshots: SubmittedPromptUiSnapshot[] = []
  const clearedBusyAgents: Array<string | null | undefined> = []
  const submittingAgentIds: Array<string | null> = []
  const submittingValues: boolean[] = []
  const workingValues: boolean[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const logErrors: Array<{ message: string; fields: Record<string, unknown> }> = []
  const fatalErrors: string[] = []
  let clearPromptCount = 0
  let commandCenterClearCount = 0
  let updateChromeCount = 0

  const controller = createProviderNamespaceSubmitController({
    getFocusedProvider: () => options.focusedProvider ?? "opencode",
    workflowScreenShowing: () => false,
    getPendingAttachmentCount: () => 0,
    waitForPendingAgentFocusTransition: async () => {},
    getFocusedAgentId: () => options.focusedAgentId ?? "agent-1",
    hasAgent: options.hasAgent ?? ((agentId) => agentId === "agent-1"),
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    getAttachment: () => options.attachment === undefined ? { id: "attachment-1", session_id: "session-1" } : options.attachment,
    getSessionId: () => "session-1",
    clearPromptText: () => {
      clearPromptCount += 1
    },
    beginSubmittedPromptUi: (rawPrompt) => ({ rawPrompt, attachments: [], sessionId: "session-1" }),
    renderPromptTranscript: (prompt) => `${prompt}\n`,
    appendUserPrompt: (text, agentId) => {
      appendedPrompts.push({ text, agentId })
    },
    submitProviderNamespacePrompt: async (attachmentId, targetAgentId, prompt) => {
      submissions.push({ attachmentId, targetAgentId, prompt })
      if (options.submitProviderNamespacePrompt) {
        return options.submitProviderNamespacePrompt(attachmentId, targetAgentId, prompt)
      }
      return promptSubmissionResult("session-submitted", "agent-submitted")
    },
    applySessionState: (session) => {
      appliedSessions.push(session)
    },
    setStreamingAgentId: (agentId) => {
      streamingAgentIds.push(agentId)
    },
    setWorking: (working) => {
      workingValues.push(working)
    },
    updateSessionChrome: () => {
      updateChromeCount += 1
    },
    recordPromptAreaHistoryEntry: (sessionId, rawPrompt) => {
      recordedHistory.push({ sessionId, rawPrompt })
    },
    clearCommandCenter: () => {
      commandCenterClearCount += 1
    },
    restoreFailedPromptUi: (snapshot) => {
      if (snapshot) {
        restoredSnapshots.push(snapshot)
      }
      return Boolean(snapshot)
    },
    getSubmittingAgentId: () => "agent-busy",
    clearAgentBusy: (agentId) => {
      clearedBusyAgents.push(agentId)
    },
    setSubmittingAgentId: (agentId) => {
      submittingAgentIds.push(agentId)
    },
    setSubmitting: (submitting) => {
      submittingValues.push(submitting)
    },
    setFatalError: (message) => {
      fatalErrors.push(message)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    logError: (message, fields) => {
      logErrors.push({ message, fields })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    submissions: () => submissions,
    appendedPrompts: () => appendedPrompts,
    appliedSessions: () => appliedSessions,
    streamingAgentIds: () => streamingAgentIds,
    recordedHistory: () => recordedHistory,
    restoredSnapshots: () => restoredSnapshots,
    clearedBusyAgents: () => clearedBusyAgents,
    submittingAgentIds: () => submittingAgentIds,
    submittingValues: () => submittingValues,
    workingValues: () => workingValues,
    footerMessages: () => footerMessages,
    logErrors: () => logErrors,
    fatalErrors: () => fatalErrors,
    clearPromptCount: () => clearPromptCount,
    commandCenterClearCount: () => commandCenterClearCount,
    updateChromeCount: () => updateChromeCount,
  }
}

function promptSubmissionResult(sessionId: string, targetAgentId: string | null): PromptSubmissionResult {
  return {
    payload: {
      outcome: {},
      session: runtimeSession(sessionId),
    },
    targetAgentId,
    outcomeName: "PromptSubmitted",
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

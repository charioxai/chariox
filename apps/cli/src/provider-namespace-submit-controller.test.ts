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
  const harness = createHarness({
    submitProviderNamespacePrompt: async () => ({
      payload: {
        outcome: {
          Started: {
            prompt: {
              id: "prompt-started",
              source_attachment_id: "attachment-started",
              target_agent_id: "agent-submitted",
              prompt: "/session list\n",
              status: "running",
              prompt_origin: " External ",
            },
          },
        },
        session: runtimeSession("session-submitted"),
        agent_activity: {},
        agent_activity_revision: 1,
      },
      targetAgentId: "agent-submitted",
      outcomeName: "PromptSubmitted",
    }),
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "/session list\n",
  }])
  assert.deepEqual(harness.appendedPrompts(), [{
    text: "/opencode session list\n",
    agentId: "agent-submitted",
    promptId: "prompt-started",
    sourceAttachmentId: "attachment-started",
    promptOrigin: "external",
  }])
  assert.equal(harness.appliedSessions().at(-1)?.id, "session-submitted")
  assert.deepEqual(harness.streamingAgentIds(), ["agent-submitted"])
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "/opencode session list" }])
  assert.equal(harness.commandCenterClearCount(), 1)
  assert.equal(harness.workingValues().at(-1), true)
})

test("provider namespace submit projects queued runtime state from active session work", async () => {
  const harness = createHarness({
    focusedAgentId: "agent-queued",
    hasAgent: (agentId) => agentId === "agent-active" || agentId === "agent-queued",
    submitProviderNamespacePrompt: async () => ({
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", {
          agents: [agent("agent-active"), agent("agent-queued")],
          prompt_states: {
            "agent-active": {
              active_prompt: {
                id: "prompt-active",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-active",
                prompt: "running",
                status: "running",
              },
              queued_prompts: [],
            },
            "agent-queued": {
              active_prompt: null,
              queued_prompts: [{
                id: "prompt-queued",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-queued",
                prompt: "/session list",
                status: "queued",
              }],
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
      targetAgentId: "agent-queued",
      outcomeName: "Queued",
    }),
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.deepEqual(harness.appendedPrompts(), [])
  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
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
  assert.deepEqual(harness.appendedPrompts(), [{ text: "/opencode session list\n", agentId: "agent-submitted" }])
})

test("provider namespace submit restores UI after submission failure", async () => {
  const harness = createHarness({
    submitProviderNamespacePrompt: async () => {
      throw new Error("provider failed")
    },
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.equal(harness.logErrors().at(-1)?.message, "provider namespace command failed")
  assert.deepEqual(harness.appendedPrompts(), [])
  assert.equal(harness.restoredSnapshots().at(-1)?.rawPrompt, "/opencode session list")
  assert.deepEqual(harness.clearedBusyAgents(), ["agent-busy"])
  assert.deepEqual(harness.submittingAgentIds(), [null])
  assert.deepEqual(harness.submittingValues(), [false])
  assert.equal(harness.workingValues().at(-1), false)
  assert.equal(harness.fatalErrors().at(-1), "provider failed")
})

test("provider namespace submit failure preserves active session runtime state", async () => {
  const harness = createHarness({
    session: runtimeSession("session-1", {
      agents: [agent("agent-active")],
      prompt_states: {
        "agent-active": {
          active_prompt: {
            id: "prompt-active",
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-active",
            prompt: "running",
            status: "running",
          },
          queued_prompts: [],
        },
      },
    }),
    submitProviderNamespacePrompt: async () => {
      throw new Error("provider failed")
    },
  })

  assert.equal(await harness.controller.submit("/opencode session list"), true)

  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
  assert.equal(harness.workingValues().at(-1), true)
})

function createHarness(options: {
  focusedProvider?: "opencode" | "codex" | "claude-headless" | "claude-p" | null
  focusedAgentId?: string | null
  hasAgent?: (agentId: string) => boolean
  attachment?: RuntimeAttachment | null
  session?: RuntimeSession
  submitProviderNamespacePrompt?: ProviderNamespaceSubmitControllerDeps["submitProviderNamespacePrompt"]
} = {}) {
  const submissions: Array<{ attachmentId: string; targetAgentId: string | null; prompt: string }> = []
  const appendedPrompts: Array<{
    text: string
    agentId: string | null | undefined
    promptId?: string | null
    sourceAttachmentId?: string | null
    promptOrigin?: string | null
  }> = []
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
    getSession: () => options.session ?? runtimeSession("session-1", { agents: [agent("agent-1")] }),
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
    appendUserPrompt: (text, agentId, metadata) => {
      appendedPrompts.push({
        text,
        agentId,
        ...(metadata?.promptId !== undefined ? { promptId: metadata.promptId } : {}),
        ...(metadata?.sourceAttachmentId !== undefined ? { sourceAttachmentId: metadata.sourceAttachmentId } : {}),
        ...(metadata?.promptOrigin !== undefined ? { promptOrigin: metadata.promptOrigin } : {}),
      })
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
      agent_activity: {},
      agent_activity_revision: 1,
    },
    targetAgentId,
    outcomeName: "PromptSubmitted",
  }
}

function runtimeSession(id: string, overrides: Partial<RuntimeSession> = {}): RuntimeSession {
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
    ...overrides,
  }
}

function agent(id: string): RuntimeSession["agents"][number] {
  return {
    id,
    agent_ref: id,
    session_id: "session-submitted",
    alias: id,
    provider: "opencode",
    model: null,
    worktree_id: "/workspace/tree",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}

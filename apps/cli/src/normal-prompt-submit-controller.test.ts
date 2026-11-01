import assert from "node:assert/strict"
import test from "node:test"

import type { PromptAttachmentPart, RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import { createPromptSubmitCoordinator } from "./prompt-submit-coordinator.js"
import { NO_FOCUS_AGENT_MESSAGE } from "./cli-runtime-tuning.js"
import {
  createNormalPromptSubmitController,
  type NormalPromptSubmitControllerDeps,
} from "./normal-prompt-submit-controller.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import type { PromptSubmissionResult } from "./prompt-runtime-api.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"
import { createPromptSubmissionUiController } from "./prompt-submission-ui-controller.js"

test("normal prompt submit requires an attachment", async () => {
  const harness = createHarness({ attachment: null })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "No session attached.")
  assert.equal(harness.clearPromptCount(), 1)
  assert.deepEqual(harness.submissions(), [])
})

test("normal prompt submit prepares attachments, submits, and records history", async () => {
  const harness = createHarness({
    pendingAttachments: [pendingAttachment("file-1")],
    inlineLocalFiles: true,
    submitPrompt: async () => promptSubmissionResult("session-submitted", "agent-submitted", "PromptSubmitted"),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.preparedAttachments(), [{
    attachments: [{ url: "/tmp/file.txt", mime: "text/plain", filename: "file.txt" }],
    inlineLocalFiles: true,
  }])
  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "hello\n",
    attachments: [{ url: "/tmp/file.txt", mime: "text/plain", filename: "file.txt", contents_base64: "ZmlsZQ==" }],
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "hello\n", agentId: "agent-submitted" }])
  assert.equal(harness.appliedSessions().at(-1)?.id, "session-submitted")
  assert.deepEqual(harness.streamingAgentIds(), ["agent-submitted"])
  assert.deepEqual(harness.statusLines(), ["Prompt submitted."])
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "hello" }])
})

test("normal prompt submit lets the kernel route an alias while rendering only the prompt body", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-reviewer", "PromptSubmitted"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          focused_agent_id: "agent-reviewer",
          agents: [agent("agent-1"), agent("agent-reviewer")],
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("@reviewer inspect package.json")

  assert.equal(harness.submissions().at(-1)?.prompt, "@reviewer inspect package.json\n")
  assert.deepEqual(harness.appendedPrompts(), [{
    text: "inspect package.json\n",
    agentId: "agent-reviewer",
  }])
  assert.equal(harness.appliedSessions().at(-1)?.focused_agent_id, "agent-reviewer")
})

test("normal prompt submit appends prompt acknowledgement metadata", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      payload: {
        outcome: {
          Started: {
            prompt: {
              id: "prompt-started",
              source_attachment_id: "attachment-started",
              target_agent_id: "agent-1",
              prompt: "hello\n",
              status: "running",
              prompt_origin: " External ",
            },
          },
        },
        session: runtimeSession("session-submitted", "prompt-started"),
        agent_activity: {},
        agent_activity_revision: 1,
      },
      targetAgentId: "agent-1",
      outcomeName: "PromptSubmitted",
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.appendedPrompts(), [{
    text: "hello\n",
    agentId: "agent-1",
    promptId: "prompt-started",
    sourceAttachmentId: "attachment-started",
    promptOrigin: "external",
  }])
})

test("normal prompt submit drops stale focused agent ids", async () => {
  const harness = createHarness({
    focusedAgentId: "old-agent",
    session: runtimeSession("session-1", null, { focused_agent_id: "old-agent", agents: [agent("agent-1")] }),
    hasAgent: (agentId) => agentId === "agent-1",
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.submissions(), [{
    attachmentId: "attachment-1",
    targetAgentId: null,
    prompt: "hello\n",
    attachments: [],
  }])
  assert.deepEqual(harness.appendedPrompts(), [{ text: "hello\n", agentId: null }])
})

test("normal prompt submit keeps the prompt when the session has no focus agent", async () => {
  const harness = createHarness({
    focusedAgentId: null,
    session: runtimeSession("session-1", null, { focused_agent_id: null, agents: [agent("agent-1")] }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.submissions(), [])
  assert.equal(harness.clearPromptCount(), 0)
  assert.deepEqual(harness.fatalErrors(), [])
  assert.deepEqual(harness.footerMessages(), [{ message: NO_FOCUS_AGENT_MESSAGE, tone: "error" }])

  await harness.controller.submit("@agent-1 hello")

  assert.equal(harness.submissions().length, 1, "an @alias prompt still goes to the kernel, which routes it")
})

test("normal prompt submit reports queued status with active prompt id", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", null, "Queued", "prompt-active"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", "prompt-active", {
          agents: [agent("agent-1")],
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello\n")

  assert.deepEqual(harness.statusLines(), ["Prompt queued behind prompt-active."])
  assert.equal(harness.submissions().at(-1)?.prompt, "hello\n")
  assert.deepEqual(harness.appendedPrompts(), [])
})

test("normal prompt submit projects queued runtime state from active session work", async () => {
  const harness = createHarness({
    focusedAgentId: "agent-queued",
    hasAgent: (agentId) => agentId === "agent-active" || agentId === "agent-queued",
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-queued", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
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
                prompt: "hello",
                status: "queued",
              }],
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.appendedPrompts(), [])
  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
  assert.equal(harness.workingValues().at(-1), true)
})

test("normal prompt submit reports queued status from per-agent active prompt state", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-1", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          agents: [agent("agent-1")],
          prompt_states: {
            "agent-1": {
              active_prompt: {
                id: "prompt-active-agent",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "running",
                status: "running",
              },
              queued_prompts: [{
                id: "prompt-queued-agent",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "hello",
                status: "queued",
              }],
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.statusLines(), ["Prompt queued behind prompt-active-agent."])
  assert.deepEqual(harness.appendedPrompts(), [])
})

test("normal prompt submit logs projected queued prompt counts", async () => {
  const harness = createHarness({
    submitPrompt: async () => ({
      ...promptSubmissionResult("session-submitted", "agent-1", "Queued"),
      payload: {
        outcome: {},
        session: runtimeSession("session-submitted", null, {
          agents: [agent("agent-1")],
          prompt_states: {
            "agent-1": {
              active_prompt: null,
              queued_prompts: [{
                id: "stale-queued",
                source_attachment_id: "attachment-1",
                target_agent_id: "agent-1",
                prompt: "stale",
                status: "queued",
              }],
            },
          },
          agent_activity: {
            "agent-1": {
              status: "working",
              prompt_status: "queued",
              busy: true,
              queued_prompt_count: 2,
              unread_idle_output: false,
            },
          },
        }),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }),
  })

  await harness.controller.submit("hello")

  assert.equal(harness.logInfos().find((entry) => entry.message === "prompt submitted")?.fields.queued_prompts, 2)
})

test("normal prompt submit restores UI after submit failure", async () => {
  const harness = createHarness({
    submitPrompt: async () => {
      throw new Error("submit failed")
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.logErrors().at(-1)?.message, "prompt submission failed")
  assert.equal(harness.restoredSnapshots().at(-1)?.rawPrompt, "hello")
  assert.deepEqual(harness.clearedBusyAgents(), ["agent-busy"])
  assert.deepEqual(harness.submittingAgentIds(), [null])
  assert.deepEqual(harness.submittingValues(), [false])
  assert.equal(harness.workingValues().at(-1), false)
  assert.equal(harness.fatalErrors().at(-1), "submit failed")
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "submit failed",
    tone: "error",
  })
})

test("normal prompt submit failure preserves active session runtime state", async () => {
  const harness = createHarness({
    session: runtimeSession("session-1", null, {
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
    submitPrompt: async () => {
      throw new Error("submit failed")
    },
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.streamingAgentIds(), ["agent-active"])
  assert.equal(harness.workingValues().at(-1), true)
})

for (const pending of ["focus", "attachments"] as const) {
  test(`rapid Enter captures separate drafts while ${pending} preparation is pending`, async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    let draft = "first task"
    let attachments = [{ ...pendingAttachment("first"), filename: "first.txt" }]
    const harness = createHarness({
      getPendingAttachments: () => attachments,
      waitForPendingAgentFocusTransition: () => pending === "focus" ? gate : Promise.resolve(),
      preparePromptAttachmentsForSubmit: async (parts) => {
        if (pending === "attachments") await gate
        return parts
      },
      beginSubmittedPromptUi: (rawPrompt) => {
        const snapshot = { rawPrompt, attachments, sessionId: "session-1" }
        draft = ""
        attachments = []
        return snapshot
      },
    })
    const coordinator = createPromptSubmitCoordinator({
      getPromptText: () => draft,
      ensureBackgroundPollersStarted: () => {},
      getPendingAttachmentCount: () => attachments.length,
      clearPromptText: () => { draft = "" },
      workflowScreenShowing: () => false,
      submitWorkspaceShellCommand: async () => {},
      workflowNodeInstructionsEditorOpen: () => false,
      submitSlashCommand: async () => false,
      submitProviderNamespacePrompt: async () => false,
      isAttached: () => true,
      submitWorkflowPrompt: async () => {},
      submitNormalPrompt: (rawPrompt) => harness.controller.submit(rawPrompt),
      flashFooter: () => {},
    })
    const first = coordinator.submit()
    assert.equal(draft, "", "Enter consumes its draft before any asynchronous routing")
    draft = "genuine queued task"
    attachments = [{ ...pendingAttachment("second"), filename: "second.txt" }]
    const second = coordinator.submit()
    draft = "new unsent draft"
    release()
    await Promise.all([first, second])
    assert.equal(draft, "new unsent draft", "completion preserves the next draft")
    assert.deepEqual(harness.submissions().map(({ prompt, attachments }) => ({
      prompt, attachments: attachments.map((part) => part.filename),
    })), [
      { prompt: "first task\n", attachments: ["first.txt"] },
      { prompt: "genuine queued task\n", attachments: ["second.txt"] },
    ])
    assert.deepEqual(harness.recordedHistory().map((entry) => entry.rawPrompt), ["first task", "genuine queued task"])
  })
}

function createHarness(options: {
  getPendingAttachments?: NormalPromptSubmitControllerDeps["getPendingAttachments"]
  waitForPendingAgentFocusTransition?: NormalPromptSubmitControllerDeps["waitForPendingAgentFocusTransition"]
  preparePromptAttachmentsForSubmit?: NormalPromptSubmitControllerDeps["preparePromptAttachmentsForSubmit"]
  beginSubmittedPromptUi?: NormalPromptSubmitControllerDeps["beginSubmittedPromptUi"]
  restoreFailedPromptUi?: NormalPromptSubmitControllerDeps["restoreFailedPromptUi"]
  promptBlockReason?: NormalPromptSubmitControllerDeps["promptBlockReason"]
  onPromptRejected?: NormalPromptSubmitControllerDeps["onPromptRejected"]
  attachment?: RuntimeAttachment | null
  pendingAttachments?: PendingPromptAttachment[]
  inlineLocalFiles?: boolean
  focusedAgentId?: string | null
  session?: RuntimeSession
  hasAgent?: (agentId: string) => boolean
  submitPrompt?: NormalPromptSubmitControllerDeps["submitPrompt"]
} = {}) {
  const preparedAttachments: Array<{ attachments: PromptAttachmentPart[]; inlineLocalFiles: boolean }> = []
  const submissions: Array<{
    attachmentId: string
    targetAgentId: string | null
    prompt: string
    attachments: PromptAttachmentPart[]
  }> = []
  const appendedPrompts: Array<{
    text: string
    agentId: string | null | undefined
    promptId?: string | null
    sourceAttachmentId?: string | null
    promptOrigin?: string | null
  }> = []
  const appliedSessions: RuntimeSession[] = []
  const streamingAgentIds: Array<string | null> = []
  const statusLines: string[] = []
  const recordedHistory: Array<{ sessionId: string; rawPrompt: string }> = []
  const restoredSnapshots: SubmittedPromptUiSnapshot[] = []
  const clearedBusyAgents: Array<string | null | undefined> = []
  const submittingAgentIds: Array<string | null> = []
  const submittingValues: boolean[] = []
  const workingValues: boolean[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const logErrors: Array<{ message: string; fields: Record<string, unknown> }> = []
  const logInfos: Array<{ message: string; fields: Record<string, unknown> }> = []
  const fatalErrors: string[] = []
  let clearPromptCount = 0

  const controller = createNormalPromptSubmitController({
    ...(options.promptBlockReason ? { promptBlockReason: options.promptBlockReason } : {}),
    ...(options.onPromptRejected ? {onPromptRejected: options.onPromptRejected} : {}),
    getPendingAttachments: options.getPendingAttachments ?? (() => options.pendingAttachments ?? []),
    waitForPendingAgentFocusTransition: options.waitForPendingAgentFocusTransition ?? (async () => {}),
    getFocusedAgentId: () => options.focusedAgentId === undefined ? "agent-1" : options.focusedAgentId,
    getSession: () => options.session ?? runtimeSession("session-1", null, { agents: [agent("agent-1")] }),
    hasAgent: options.hasAgent ?? ((agentId) => agentId === "agent-1"),
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    getAttachment: () => options.attachment === undefined ? { id: "attachment-1", session_id: "session-1" } : options.attachment,
    getSessionId: () => "session-1",
    clearPromptText: () => {
      clearPromptCount += 1
    },
    shouldInlineLocalFiles: () => options.inlineLocalFiles ?? false,
    preparePromptAttachmentsForSubmit: async (attachments, transferOptions) => {
      preparedAttachments.push({ attachments, inlineLocalFiles: transferOptions.inlineLocalFiles })
      if (options.preparePromptAttachmentsForSubmit) return options.preparePromptAttachmentsForSubmit(attachments, transferOptions)
      return attachments.map((attachment) => ({ ...attachment, contents_base64: "ZmlsZQ==" }))
    },
    beginSubmittedPromptUi: options.beginSubmittedPromptUi ?? ((rawPrompt) => ({ rawPrompt, attachments: [], sessionId: "session-1" })),
    renderPromptTranscript: (prompt) => prompt,
    appendUserPrompt: (text, agentId, metadata) => {
      appendedPrompts.push({
        text,
        agentId,
        ...(metadata?.promptId !== undefined ? { promptId: metadata.promptId } : {}),
        ...(metadata?.sourceAttachmentId !== undefined ? { sourceAttachmentId: metadata.sourceAttachmentId } : {}),
        ...(metadata?.promptOrigin !== undefined ? { promptOrigin: metadata.promptOrigin } : {}),
      })
    },
    submitPrompt: async (attachmentId, targetAgentId, prompt, attachments) => {
      submissions.push({ attachmentId, targetAgentId, prompt, attachments })
      return options.submitPrompt
        ? options.submitPrompt(attachmentId, targetAgentId, prompt, attachments)
        : promptSubmissionResult("session-submitted", targetAgentId, "PromptSubmitted")
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
    updateSessionChrome: () => {},
    setStatusLine: (line) => {
      statusLines.push(line)
    },
    recordPromptAreaHistoryEntry: (sessionId, rawPrompt) => {
      recordedHistory.push({ sessionId, rawPrompt })
    },
    restoreFailedPromptUi: (snapshot) => {
      if (snapshot) {
        restoredSnapshots.push(snapshot)
      }
      return options.restoreFailedPromptUi?.(snapshot) ?? Boolean(snapshot)
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
    logInfo: (message, fields) => {
      logInfos.push({ message, fields })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    preparedAttachments: () => preparedAttachments,
    submissions: () => submissions,
    appendedPrompts: () => appendedPrompts,
    appliedSessions: () => appliedSessions,
    streamingAgentIds: () => streamingAgentIds,
    statusLines: () => statusLines,
    recordedHistory: () => recordedHistory,
    restoredSnapshots: () => restoredSnapshots,
    clearedBusyAgents: () => clearedBusyAgents,
    submittingAgentIds: () => submittingAgentIds,
    submittingValues: () => submittingValues,
    workingValues: () => workingValues,
    footerMessages: () => footerMessages,
    logInfos: () => logInfos,
    logErrors: () => logErrors,
    fatalErrors: () => fatalErrors,
    clearPromptCount: () => clearPromptCount,
  }
}

function agent(id: string): RuntimeSession["agents"][number] {
  return {
    id,
    agent_ref: id,
    session_id: "session-submitted",
    alias: id,
    provider: "codex",
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

function pendingAttachment(id: string): PendingPromptAttachment {
  return {
    id,
    url: "/tmp/file.txt",
    mime: "text/plain",
    filename: "file.txt",
    kind: "text",
    token: "[file 1]",
  }
}

function promptSubmissionResult(
  sessionId: string,
  targetAgentId: string | null,
  outcomeName: string,
  activePromptId: string | null = null,
): PromptSubmissionResult {
  return {
    payload: {
      outcome: {},
      session: runtimeSession(sessionId, activePromptId),
      agent_activity: {},
      agent_activity_revision: 1,
    },
    targetAgentId,
    outcomeName,
  }
}

function runtimeSession(
  id: string,
  activePromptId: string | null,
  overrides: Partial<RuntimeSession> = {},
): RuntimeSession {
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
    ...(activePromptId
      ? { prompt_states: {
        "agent-1": {
          active_prompt: {
            id: activePromptId,
            source_attachment_id: "attachment-1",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "running",
          },
          queued_prompts: [],
        },
      } }
      : {}),
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

 test("MP-08 / MP-11 unsigned focused account preserves draft and attachments before provider work", async () => {
  const harness = createHarness({ pendingAttachments: [pendingAttachment("file-1")], promptBlockReason: () => "codex-1 needs sign-in. Sign in through Provider Accounts, or select signed-in account chariox." })
  await harness.controller.submit("hi")
  assert.equal(harness.clearPromptCount(), 0)
  assert.deepEqual(harness.preparedAttachments(), [])
  assert.deepEqual(harness.submissions(), [])
  assert.deepEqual(harness.appendedPrompts(), [])
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /needs sign-in.*chariox/)
})

test("MP-08 / MP-10 / MP-11 account rejection restores the real cleared editor and attachments", async () => {
  let draft = "Review this attached note"
  let attachments = [pendingAttachment("file-1")]
  const original = attachments.map(file => ({ ...file }))
  const persisted: string[] = []
  const ui = createPromptSubmissionUiController({
    getPromptText: () => draft,
    getSessionId: () => "session-1",
    getPendingAttachments: () => attachments,
    resetPromptHistoryNavigation: () => {},
    clearDraftPersistQueue: () => {},
    clearPromptText: () => { draft = "" },
    setPromptText: text => { draft = text },
    syncPromptTextSnapshot: () => {},
    clearPendingAttachments: () => { attachments = [] },
    setPendingAttachments: files => { attachments = files },
    refreshAttachmentHighlights: () => {},
    syncCommandCenter: () => {},
    retainPromptFocus: () => {},
    clearCommandCenter: () => {},
    schedulePromptDraftPersist: (_session, text) => { persisted.push(text) },
    updateSessionChrome: () => {},
  })
  const harness = createHarness({
    getPendingAttachments: () => attachments,
    beginSubmittedPromptUi: prompt => ui.begin(prompt),
    restoreFailedPromptUi: snapshot => ui.restore(snapshot),
    promptBlockReason: () => "Selected account needs sign-in",
  })
  await harness.controller.submit(draft)
  assert.equal(draft, "Review this attached note")
  assert.deepEqual(attachments, original)
  assert.deepEqual(persisted, ["", "Review this attached note"])
  assert.deepEqual(harness.submissions(), [])
  assert.deepEqual(harness.recordedHistory(), [])
})

test("MP-08/MP-10/MP-11 kernel auth rejection retains the addressed alias instead of focus", async () => {
  const targets: unknown[] = []
  const h = createHarness({
    session: runtimeSession("session-1", null, { agents: [agent("agent-1"), agent("agent-b")] }),
    promptBlockReason: () => null,
    onPromptRejected: target => targets.push(target),
    submitPrompt: async () => { throw Error("selected codex account b is not authenticated") },
  })
  await h.controller.submit("@agent-b read README")
  assert.deepEqual(targets, [{sessionId: "session-1", agentId: "agent-b"}])
  assert.equal(h.restoredSnapshots()[0]?.rawPrompt, "@agent-b read README")
  assert.equal(h.footerMessages().at(-1)?.message, "selected codex account b is not authenticated")
})

// MP-08 / MP-09 / MP-10 / MP-11 EA2-80: Continue uses ordinary submission once.
function retrySession(kind = "interrupted_by_restart", retryable = true, busy = false): RuntimeSession {
  return runtimeSession("session-1", null, { agents: [agent("agent-1")], agent_activity: {
    "agent-1": { status: busy ? "working" : kind === "completed" || kind === "cancelled" ? "idle" : "error",
      busy, prompt_status: busy ? "running" : "none", unread_idle_output: false,
      last_completed_turn: { turn_id: "turn-1", outcome: { kind, retryable, reason: "Kernel restarted" } },
    },
  } } as unknown as Partial<RuntimeSession>)
}

for (const [kind, retryable, busy] of [
  ["completed", true, false], ["cancelled", true, false], ["other", false, false],
  ["interrupted_by_restart", true, true],
] as const) test(`Continue refuses ${kind} retryable=${retryable} busy=${busy}`, async () => {
  const harness = createHarness({ session: retrySession(kind, retryable, busy) })
  await harness.controller.continueLatest()
  assert.deepEqual(harness.submissions(), [])
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /No retryable turn/)
})

test("Continue without a previous outcome is refused", async () => {
  const harness = createHarness()
  await harness.controller.continueLatest()
  assert.deepEqual(harness.submissions(), [])
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /No retryable turn/)
})

test("Continue submits once during acceptance, then refuses the completed turn", async () => {
  const current = retrySession()
  let release!: () => void
  const gate = new Promise<void>(resolve => { release = resolve })
  const harness = createHarness({ session: current, submitPrompt: async () => {
    await gate
    current.agent_activity = retrySession("completed", false).agent_activity!
    return promptSubmissionResult("session-1", "agent-1", "PromptSubmitted")
  } })
  const first = harness.controller.continueLatest()
  await harness.controller.continueLatest()
  release()
  await first
  assert.deepEqual(harness.submissions(), [{ attachmentId: "attachment-1", targetAgentId: "agent-1", prompt: "Continue\n", attachments: [] }])
  await harness.controller.continueLatest()
  assert.equal(harness.submissions().length, 1)
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /No retryable turn/)
})


test("Continue rechecks the outcome after an asynchronous focus transition", async () => {
  const current = retrySession()
  const harness = createHarness({ session: current, waitForPendingAgentFocusTransition: async () => {
    current.agent_activity = retrySession("interrupted_by_restart", true, true).agent_activity!
  } })
  await harness.controller.continueLatest()
  assert.deepEqual(harness.submissions(), [])
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /No retryable turn/)
})


test("Continue refuses if another turn settles during attachment preparation", async () => {
  const current = retrySession()
  const harness = createHarness({ session: current, preparePromptAttachmentsForSubmit: async () => {
    current.agent_activity!["agent-1"]!.last_completed_turn!.turn_id = "different-turn"
    return []
  } })
  await harness.controller.continueLatest()
  assert.deepEqual(harness.submissions(), [])
  assert.match(harness.footerMessages().at(-1)?.message ?? "", /No retryable turn/)
  assert.equal(harness.restoredSnapshots().length, 1, "a stale action must restore its consumed draft and attachments")
  assert.equal(harness.restoredSnapshots()[0]?.rawPrompt, "/continue")
})


test("Continue consumes the command before waiting so the next draft survives", async () => {
  let release!: () => void
  let began = false
  const gate = new Promise<void>(resolve => { release = resolve })
  const harness = createHarness({ session: retrySession(), waitForPendingAgentFocusTransition: () => gate,
    beginSubmittedPromptUi: rawPrompt => { began = true; return { rawPrompt, attachments: [], sessionId: "session-1" } },
  })
  const pending = harness.controller.continueLatest()
  assert.equal(began, true, "consume this Enter before any focus wait")
  release()
  await pending
  assert.equal(harness.submissions().length, 1)
})

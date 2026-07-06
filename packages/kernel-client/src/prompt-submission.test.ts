import assert from "node:assert/strict"
import test from "node:test"

import {
  formatPromptSubmissionBody,
  formatPromptSubmissionStatusLine,
  detachedPromptSubmitDecision,
  promptSubmissionFailureRuntimeState,
  promptSubmissionFailureTransition,
  promptSubmissionRuntimeState,
  promptSubmitPreparationDecision,
  promptSubmissionSuccessTransition,
  promptSubmissionAttachmentsToParts,
  resolvePromptSubmissionTargetAgentId,
} from "./prompt-submission.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("formatPromptSubmissionBody terminates non-empty prompts", () => {
  assert.equal(formatPromptSubmissionBody("hello"), "hello\n")
  assert.equal(formatPromptSubmissionBody("hello\n"), "hello\n")
  assert.equal(formatPromptSubmissionBody("  "), "")
})

test("prompt submit preparation decision handles empty, workspace, editor, and slash policy", () => {
  assert.deepEqual(promptSubmitPreparationDecision({
    rawPrompt: "  ",
    pendingAttachmentCount: 0,
    workflowScreenShowing: false,
    workspaceShellCommand: false,
    workflowNodeInstructionsEditorOpen: false,
    workflowCommandInput: false,
  }), { action: "clear_empty" })
  assert.deepEqual(promptSubmitPreparationDecision({
    rawPrompt: "@ status",
    pendingAttachmentCount: 0,
    workflowScreenShowing: true,
    workspaceShellCommand: true,
    workflowNodeInstructionsEditorOpen: false,
    workflowCommandInput: false,
  }), { action: "workspace_shell" })
  assert.deepEqual(promptSubmitPreparationDecision({
    rawPrompt: "new instructions",
    pendingAttachmentCount: 0,
    workflowScreenShowing: false,
    workspaceShellCommand: false,
    workflowNodeInstructionsEditorOpen: true,
    workflowCommandInput: false,
  }), { action: "instructions_editor_open" })
  assert.deepEqual(promptSubmitPreparationDecision({
    rawPrompt: "/session list",
    pendingAttachmentCount: 0,
    workflowScreenShowing: true,
    workspaceShellCommand: false,
    workflowNodeInstructionsEditorOpen: false,
    workflowCommandInput: true,
  }), {
    action: "continue",
    trimmedPrompt: "/session list",
    allowSlashCommandSubmission: true,
  })
})

test("detached prompt submit decision blocks unsupported input and routes bootstrap outcomes", () => {
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "/agent focus 1",
    pendingAttachmentCount: 0,
  }), { action: "flash_start_or_join_session" })
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "hello",
    pendingAttachmentCount: 1,
  }), { action: "flash_attachments_require_session" })
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "hello",
    pendingAttachmentCount: 0,
  }), { action: "bootstrap" })
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "hello",
    pendingAttachmentCount: 0,
    bootstrapResult: "handled",
  }), { action: "keep_bootstrap_handled" })
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "hello",
    pendingAttachmentCount: 0,
    bootstrapResult: "bootstrapped",
    attachedAfterBootstrap: true,
  }), { action: "submit_bootstrapped_prompt" })
  assert.deepEqual(detachedPromptSubmitDecision({
    trimmedPrompt: "hello",
    pendingAttachmentCount: 0,
    bootstrapResult: "unhandled",
  }), { action: "flash_no_session_and_clear" })
})

test("promptSubmissionAttachmentsToParts strips prompt-only attachment fields", () => {
  const attachments = [
    {
      id: "attachment-1",
      url: "file:///tmp/a.txt",
      mime: "text/plain",
      filename: "a.txt",
      kind: "text",
      token: "[file 1]",
    },
  ]

  assert.deepEqual(promptSubmissionAttachmentsToParts(attachments), [
    {
      url: "file:///tmp/a.txt",
      mime: "text/plain",
      filename: "a.txt",
    },
  ])
})

test("resolvePromptSubmissionTargetAgentId keeps only live requested agents", () => {
  const hasAgent = (agentId: string) => agentId === "agent-1"

  assert.equal(resolvePromptSubmissionTargetAgentId({
    requestedTargetAgentId: "agent-1",
    hasAgent,
  }), "agent-1")
  assert.equal(resolvePromptSubmissionTargetAgentId({
    requestedTargetAgentId: "stale-agent",
    hasAgent,
  }), null)
  assert.equal(resolvePromptSubmissionTargetAgentId({
    requestedTargetAgentId: null,
    hasAgent,
  }), null)
  assert.equal(resolvePromptSubmissionTargetAgentId({
    hasAgent,
  }), null)
})

test("formatPromptSubmissionStatusLine describes queued and submitted outcomes", () => {
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Queued",
      activePromptId: "prompt-1",
    }),
    "Prompt queued behind prompt-1.",
  )
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Queued",
      activePromptId: null,
    }),
    "Prompt queued behind the active turn.",
  )
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Submitted",
      activePromptId: "prompt-1",
    }),
    "Prompt submitted.",
  )
})

test("promptSubmissionRuntimeState follows session work for queued submissions", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-active" }), makeAgent({ id: "agent-queued" })],
    active_prompt: {
      id: "prompt-active",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-active",
      prompt: "running",
      status: "running",
    },
    queued_prompts: [{
      id: "prompt-queued",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-queued",
      prompt: "queued",
      status: "queued",
    }],
  })

  assert.deepEqual(promptSubmissionRuntimeState({
    session,
    outcomeName: "Queued",
    submittedTargetAgentId: "agent-queued",
  }), {
    streamingAgentId: "agent-active",
    working: true,
  })
})

test("promptSubmissionSuccessTransition projects queued prompt metadata", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-active" }), makeAgent({ id: "agent-queued" })],
    active_prompt: {
      id: "prompt-active",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-active",
      prompt: "running",
      status: "running",
    },
    queued_prompts: [{
      id: "prompt-queued",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-queued",
      prompt: "queued",
      status: "queued",
    }],
  })

  assert.deepEqual(promptSubmissionSuccessTransition({
    session,
    outcomeName: "Queued",
    submittedTargetAgentId: "agent-queued",
  }), {
    shouldAppendUserPrompt: false,
    activePromptId: null,
    queuedPromptCount: 1,
    statusLine: "Prompt queued behind the active turn.",
    streamingAgentId: "agent-active",
    working: true,
  })
})

test("promptSubmissionSuccessTransition projects started prompt metadata", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-started" })],
  })

  assert.deepEqual(promptSubmissionSuccessTransition({
    session,
    outcomeName: "Started",
    submittedTargetAgentId: "agent-started",
  }), {
    shouldAppendUserPrompt: true,
    activePromptId: null,
    queuedPromptCount: 0,
    statusLine: "Prompt submitted.",
    streamingAgentId: "agent-started",
    working: true,
  })
})

test("promptSubmissionRuntimeState does not invent streaming state for queued-only snapshots", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-queued" })],
    queued_prompts: [{
      id: "prompt-queued",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-queued",
      prompt: "queued",
      status: "queued",
    }],
  })

  assert.deepEqual(promptSubmissionRuntimeState({
    session,
    outcomeName: "Queued",
    submittedTargetAgentId: "agent-queued",
  }), {
    streamingAgentId: null,
    working: true,
  })
})

test("promptSubmissionRuntimeState allows optimistic streaming only for started submissions", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-started" })],
  })

  assert.deepEqual(promptSubmissionRuntimeState({
    session,
    outcomeName: "Started",
    submittedTargetAgentId: "agent-started",
  }), {
    streamingAgentId: "agent-started",
    working: true,
  })
})

test("promptSubmissionFailureRuntimeState follows current session work", () => {
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-active" })],
    active_prompt: {
      id: "prompt-active",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-active",
      prompt: "running",
      status: "running",
    },
  })

  assert.deepEqual(promptSubmissionFailureRuntimeState(activeSession), {
    streamingAgentId: "agent-active",
    working: true,
  })

  assert.deepEqual(promptSubmissionFailureRuntimeState(makeSession({
    agents: [makeAgent({ id: "agent-idle" })],
  })), {
    streamingAgentId: null,
    working: false,
  })
})

test("promptSubmissionFailureTransition resets submitting while preserving session runtime work", () => {
  const activeSession = makeSession({
    agents: [makeAgent({ id: "agent-active" })],
    active_prompt: {
      id: "prompt-active",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-active",
      prompt: "running",
      status: "running",
    },
  })

  assert.deepEqual(promptSubmissionFailureTransition({
    session: activeSession,
    submittingAgentId: "agent-submitting",
  }), {
    clearBusyAgentId: "agent-submitting",
    submittingAgentId: null,
    submitting: false,
    streamingAgentId: "agent-active",
    working: true,
  })

  assert.deepEqual(promptSubmissionFailureTransition({
    session: makeSession({
      agents: [makeAgent({ id: "agent-idle" })],
    }),
    submittingAgentId: null,
  }), {
    clearBusyAgentId: null,
    submittingAgentId: null,
    submitting: false,
    streamingAgentId: null,
    working: false,
  })
})

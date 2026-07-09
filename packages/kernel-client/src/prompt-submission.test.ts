import assert from "node:assert/strict"
import test from "node:test"

import {
  expectPromptSubmittedPayload,
  formatPromptSubmissionBody,
  formatPromptSubmissionStatusLine,
  detachedPromptSubmitDecision,
  promptSubmittedPromptIdFromResponse,
  promptSubmittedPayloadFromResponse,
  promptSubmissionFailureRuntimeState,
  promptSubmissionFailureTransition,
  promptSubmissionOutcomeName,
  promptSubmissionPrompt,
  promptSubmissionTranscriptMetadata,
  promptSubmissionRuntimeState,
  promptSubmitPreparationDecision,
  promptSubmissionSuccessTransition,
  promptSubmissionAttachmentsToParts,
  promptSubmissionTargetAgentId,
  resolvePromptSubmissionTargetAgentId,
} from "./prompt-submission.js"
import { promptQueueItemTranscriptMetadata } from "./transcript-entry-state.js"
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

test("prompt submitted response helpers parse acknowledgement shape", () => {
  const payload = {
    outcome: { Started: {} },
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }

  assert.equal(promptSubmittedPayloadFromResponse({ PromptSubmitted: payload }), payload)
  assert.equal(promptSubmittedPayloadFromResponse({ SessionState: {} }), null)
  assert.equal(expectPromptSubmittedPayload({ PromptSubmitted: payload }), payload)
  assert.throws(
    () => expectPromptSubmittedPayload({ SessionState: {} }),
    /unexpected response variant: expected PromptSubmitted/,
  )
  assert.equal(promptSubmissionOutcomeName(payload), "Started")
  assert.equal(promptSubmissionOutcomeName({ outcome: {} }), "unknown")
  assert.equal(promptSubmissionOutcomeName({}), "unknown")
})

test("prompt submitted prompt id helper applies projected session activity", () => {
  const stalePrompt = {
    id: "prompt-stale",
    source_attachment_id: "attachment-state",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
  }

  assert.equal(promptSubmittedPromptIdFromResponse({
    PromptSubmitted: {
      outcome: {},
      session: makeSession({
        active_prompt: stalePrompt,
        prompt_states: {
          "agent-1": {
            active_prompt: stalePrompt,
            queued_prompts: [],
          },
        },
      }),
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
        },
      },
      agent_activity_revision: 7,
    },
  }, "agent-1"), null)

  assert.equal(promptSubmittedPromptIdFromResponse({
    PromptSubmitted: {
      outcome: {},
      session: makeSession({
        prompt_states: {
          "agent-1": {
            active_prompt: stalePrompt,
            queued_prompts: [],
          },
        },
      }),
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-stale",
            status: "running",
            phase: "streaming",
          },
        },
      },
      agent_activity_revision: 8,
    },
  }, "agent-1"), "prompt-stale")
})

test("prompt submission target agent id reads outcome prompt identity", () => {
  assert.equal(promptSubmissionTargetAgentId({
    outcome: {
      Started: {
        prompt: {
          id: "prompt-started",
          source_attachment_id: "attachment-started",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "Running",
        },
      },
    },
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }), "agent-1")
  assert.equal(promptSubmissionTargetAgentId({
    outcome: {},
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }), null)
})

test("prompt submission prompt prefers outcome prompt then projected session prompt", () => {
  const outcomePrompt = {
    id: "prompt-started",
    source_attachment_id: "attachment-started",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "Running",
  }
  const statePrompt = {
    id: "prompt-state",
    source_attachment_id: "attachment-state",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
  }

  assert.deepEqual(promptSubmissionPrompt({
    outcome: {
      Started: { prompt: outcomePrompt },
    },
  }, "agent-1"), outcomePrompt)

  assert.deepEqual(promptSubmissionPrompt({
    outcome: {
      Started: { prompt: outcomePrompt },
    },
    session: makeSession({
      active_prompt: statePrompt,
    }),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), outcomePrompt)

  assert.deepEqual(promptSubmissionPrompt({
    outcome: {},
    session: makeSession({
      prompt_states: {
        "agent-1": {
          active_prompt: statePrompt,
          queued_prompts: [],
        },
      },
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-state",
            status: "running",
            phase: "streaming",
          },
        },
      },
    }),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), statePrompt)
})

test("prompt submission transcript metadata prefers outcome prompt identity", () => {
  assert.deepEqual(promptSubmissionTranscriptMetadata({
    outcome: {
      Started: {
        prompt: {
          id: "prompt-started",
          source_attachment_id: "attachment-started",
          target_agent_id: "agent-1",
          prompt: "hello",
          status: "Running",
          prompt_origin: " External ",
        },
      },
    },
    session: makeSession({
      active_prompt: {
        id: "prompt-state",
        source_attachment_id: "attachment-state",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "running",
        prompt_origin: "arroba",
      },
    }),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), {
    promptId: "prompt-started",
    sourceAttachmentId: "attachment-started",
    promptOrigin: "external",
  })

  assert.deepEqual(promptSubmissionTranscriptMetadata({
    outcome: {
      Started: {
        prompt: {
          id: "external:codex:thread-1:turn-1",
          source_attachment_id: "attachment-external",
          target_agent_id: "agent-1",
          prompt: "hello from outside",
          status: "Running",
          prompt_origin: "external",
        },
      },
    },
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), {
    promptId: "external:codex:thread-1:turn-1",
    sourceAttachmentId: "attachment-external",
    promptOrigin: "external",
  })

  assert.deepEqual(promptSubmissionTranscriptMetadata({
    outcome: {
      Started: {
        prompt: {
          id: "external:codex:thread-2:turn-2",
          source_attachment_id: "attachment-inferred",
          target_agent_id: "agent-1",
          prompt: "hello from outside",
          status: "Running",
          external_provider: "codex",
          external_provider_session_id: "thread-2",
          external_provider_turn_id: "turn-2",
        },
      },
    },
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), {
    promptId: "external:codex:thread-2:turn-2",
    sourceAttachmentId: "attachment-inferred",
    externalProvider: "codex",
    externalProviderSessionId: "thread-2",
    externalProviderTurnId: "turn-2",
  })

  assert.deepEqual(promptSubmissionTranscriptMetadata({
    outcome: {
      Started: {
        prompt: {
          id: "external:codex:thread-3:from-id",
          source_attachment_id: "attachment-explicit",
          target_agent_id: "agent-1",
          prompt: "hello from outside",
          status: "Running",
          external_provider: "opencode",
          external_provider_session_id: "thread-explicit",
          external_provider_turn_id: "turn-explicit",
        },
      },
    },
    session: makeSession(),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), {
    promptId: "external:codex:thread-3:from-id",
    sourceAttachmentId: "attachment-explicit",
    externalProvider: "opencode",
    externalProviderSessionId: "thread-explicit",
    externalProviderTurnId: "turn-explicit",
  })
})

test("prompt queue item transcript metadata preserves explicit kernel ownership only", () => {
  assert.deepEqual(promptQueueItemTranscriptMetadata({
    id: "external:codex:thread-1:turn-from-id",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "Running",
  }), {
    promptId: "external:codex:thread-1:turn-from-id",
    sourceAttachmentId: "attachment-1",
  })

  assert.deepEqual(promptQueueItemTranscriptMetadata({
    id: "prompt-explicit",
    source_attachment_id: "attachment-2",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "Running",
    prompt_origin: " External ",
    external_provider: " CODEX ",
    external_provider_session_id: " thread-2 ",
    external_provider_turn_id: " turn-2 ",
  }), {
    promptId: "prompt-explicit",
    sourceAttachmentId: "attachment-2",
    promptOrigin: "external",
    externalProvider: "codex",
    externalProviderSessionId: "thread-2",
    externalProviderTurnId: "turn-2",
  })
})

test("prompt submission transcript metadata falls back to projected session prompt", () => {
  assert.deepEqual(promptSubmissionTranscriptMetadata({
    outcome: {},
    session: makeSession({
      agents: [makeAgent({ id: "agent-1" })],
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-state",
            source_attachment_id: "attachment-state",
            target_agent_id: "agent-1",
            prompt: "hello",
            status: "running",
            prompt_origin: " External ",
          },
          queued_prompts: [],
        },
      },
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-state",
            status: "running",
            phase: "streaming",
          },
        },
      },
    }),
    agent_activity: {},
    agent_activity_revision: 1,
  }, "agent-1"), {
    promptId: "prompt-state",
    sourceAttachmentId: "attachment-state",
    promptOrigin: "external",
  })
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
          prompt: "queued",
          status: "queued",
        }],
      },
    },
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
          prompt: "queued",
          status: "queued",
        }],
      },
    },
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

test("promptSubmissionRuntimeState does not invent active working state for queued-only snapshots", () => {
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
    working: false,
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

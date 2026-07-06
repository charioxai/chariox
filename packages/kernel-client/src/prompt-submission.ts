import type { PromptAttachmentPart, RuntimeSession } from "./kernel-types.js"
import {
  sessionActivePromptIdForAgent,
} from "./session-prompt-identity.js"
import {
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionQueuedPromptCount,
} from "./session-prompt-work.js"

export type PromptSubmissionAttachmentInput = Pick<PromptAttachmentPart, "url" | "mime" | "filename">

export function formatPromptSubmissionBody(rawPrompt: string): string {
  return rawPrompt.trim() ? (rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`) : ""
}

export function promptSubmissionAttachmentsToParts(
  attachments: readonly PromptSubmissionAttachmentInput[],
): PromptAttachmentPart[] {
  return attachments.map((file) => ({
    url: file.url,
    mime: file.mime,
    filename: file.filename,
  }))
}

export function resolvePromptSubmissionTargetAgentId(options: {
  readonly requestedTargetAgentId?: string | null
  readonly hasAgent: (agentId: string) => boolean
}): string | null {
  const requestedTargetAgentId = options.requestedTargetAgentId ?? null
  return requestedTargetAgentId && options.hasAgent(requestedTargetAgentId)
    ? requestedTargetAgentId
    : null
}

export function formatPromptSubmissionStatusLine(options: {
  readonly outcomeName: string
  readonly activePromptId?: string | null
}): string {
  return options.outcomeName === "Queued"
    ? `Prompt queued behind ${options.activePromptId ?? "the active turn"}.`
    : "Prompt submitted."
}

export function promptSubmissionRuntimeState(options: {
  readonly session: RuntimeSession
  readonly outcomeName: string
  readonly submittedTargetAgentId?: string | null
}): {
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const projectedStreamingAgentId = sessionProjectedStreamingAgentId(options.session)
  if (options.outcomeName === "Queued") {
    return {
      streamingAgentId: projectedStreamingAgentId,
      working: sessionHasPromptWork(options.session),
    }
  }
  return {
    streamingAgentId: projectedStreamingAgentId ?? options.submittedTargetAgentId ?? null,
    working: sessionHasPromptWork(options.session) || options.submittedTargetAgentId != null,
  }
}

export function promptSubmissionSuccessTransition(options: {
  readonly session: RuntimeSession
  readonly outcomeName: string
  readonly submittedTargetAgentId?: string | null
}): {
  readonly shouldAppendUserPrompt: boolean
  readonly activePromptId: string | null
  readonly queuedPromptCount: number
  readonly statusLine: string
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const activePromptId = sessionActivePromptIdForAgent(options.session, options.submittedTargetAgentId)
  const runtimeState = promptSubmissionRuntimeState(options)
  return {
    shouldAppendUserPrompt: options.outcomeName !== "Queued",
    activePromptId,
    queuedPromptCount: sessionQueuedPromptCount(options.session, options.submittedTargetAgentId),
    statusLine: formatPromptSubmissionStatusLine({
      outcomeName: options.outcomeName,
      activePromptId,
    }),
    streamingAgentId: runtimeState.streamingAgentId,
    working: runtimeState.working,
  }
}

export function promptSubmissionFailureRuntimeState(session: RuntimeSession): {
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  return {
    streamingAgentId: sessionProjectedStreamingAgentId(session),
    working: sessionHasPromptWork(session),
  }
}

export function promptSubmissionFailureTransition(options: {
  readonly session: RuntimeSession
  readonly submittingAgentId?: string | null
}): {
  readonly clearBusyAgentId: string | null
  readonly submittingAgentId: null
  readonly submitting: false
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  const runtimeState = promptSubmissionFailureRuntimeState(options.session)
  return {
    clearBusyAgentId: options.submittingAgentId ?? null,
    submittingAgentId: null,
    submitting: false,
    streamingAgentId: runtimeState.streamingAgentId,
    working: runtimeState.working,
  }
}

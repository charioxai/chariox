import type { PromptAttachmentPart, RuntimeSession } from "./kernel-types.js"
import {
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
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

export function promptSubmissionFailureRuntimeState(session: RuntimeSession): {
  readonly streamingAgentId: string | null
  readonly working: boolean
} {
  return {
    streamingAgentId: sessionProjectedStreamingAgentId(session),
    working: sessionHasPromptWork(session),
  }
}

import {
  formatPromptSubmissionBody as sharedFormatPromptSubmissionBody,
  formatPromptSubmissionStatusLine as sharedFormatPromptSubmissionStatusLine,
  promptSubmissionAttachmentsToParts,
} from "@arroba/kernel-client/prompt-submission"
import type { PromptAttachmentPart } from "./cli-types.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"

export function formatPromptSubmissionBody(rawPrompt: string): string {
  return sharedFormatPromptSubmissionBody(rawPrompt)
}

export function pendingPromptAttachmentsToParts(
  attachments: readonly PendingPromptAttachment[],
): PromptAttachmentPart[] {
  return promptSubmissionAttachmentsToParts(attachments) as PromptAttachmentPart[]
}

export function formatPromptSubmissionStatusLine(options: {
  outcomeName: string
  activePromptId?: string | null
}): string {
  return sharedFormatPromptSubmissionStatusLine(options)
}

import type { PromptAttachmentPart } from "./cli-types.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"

export function formatPromptSubmissionBody(rawPrompt: string): string {
  return rawPrompt.trim() ? (rawPrompt.endsWith("\n") ? rawPrompt : `${rawPrompt}\n`) : ""
}

export function pendingPromptAttachmentsToParts(
  attachments: readonly PendingPromptAttachment[],
): PromptAttachmentPart[] {
  return attachments.map((file) => ({
    url: file.url,
    mime: file.mime,
    filename: file.filename,
  }))
}

export function formatPromptSubmissionStatusLine(options: {
  outcomeName: string
  activePromptId?: string | null
}): string {
  return options.outcomeName === "Queued"
    ? `Prompt queued behind ${options.activePromptId ?? "the active turn"}.`
    : "Prompt submitted."
}

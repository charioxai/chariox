import type { PromptAttachmentPart } from "./kernel-types.js"

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

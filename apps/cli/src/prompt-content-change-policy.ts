import {
  extractDroppedPromptAttachments,
} from "./prompt-attachments.js"
import {
  isProgrammaticPromptContentEcho,
} from "@chariox/kernel-client/prompt-history"

export type PromptContentChangeDecision =
  | {
    kind: "detached"
    nextSnapshot: string
    commandCenterText: string
  }
  | {
    kind: "programmatic"
    nextSnapshot: string
    commandCenterText: string
  }
  | {
    kind: "text"
    nextSnapshot: string
    commandCenterText: string
    syncAttachmentText: string
    resetPromptHistory: boolean
    persistDraft: PromptDraftPersistence | null
  }
  | {
    kind: "drop"
    nextPromptText: string
    commandCenterText: string
    resetPromptHistory: boolean
    persistDraft: PromptDraftPersistence | null
    files: NonNullable<ReturnType<typeof extractDroppedPromptAttachments>>["files"]
    insertAt: number
  }

export type PromptDraftPersistence = {
  sessionId: string
  text: string
}

export type PromptContentChangePolicy = {
  attached: boolean
  currentText: string
  previousSnapshot: string
  programmaticMutation: boolean
  dropPending: boolean
  promptHistoryActive: boolean
  sessionId: string | null | undefined
  cwd: string
}

export function derivePromptContentChangeDecision(
  options: PromptContentChangePolicy,
): PromptContentChangeDecision {
  if (!options.attached) {
    return {
      kind: "detached",
      nextSnapshot: options.currentText,
      commandCenterText: options.currentText,
    }
  }

  if (isProgrammaticPromptContentEcho({
    currentText: options.currentText,
    previousSnapshot: options.previousSnapshot,
    programmaticMutation: options.programmaticMutation,
    dropPending: options.dropPending,
  })) {
    return {
      kind: "programmatic",
      nextSnapshot: options.currentText,
      commandCenterText: options.currentText,
    }
  }

  const drop = extractDroppedPromptAttachments(options.previousSnapshot, options.currentText, options.cwd)
  if (drop) {
    return {
      kind: "drop",
      nextPromptText: drop.nextText,
      commandCenterText: drop.nextText,
      resetPromptHistory: options.promptHistoryActive,
      persistDraft: draftPersistence(options.sessionId, drop.nextText),
      files: drop.files,
      insertAt: drop.insertAt,
    }
  }

  return {
    kind: "text",
    nextSnapshot: options.currentText,
    commandCenterText: options.currentText,
    syncAttachmentText: options.currentText,
    resetPromptHistory: options.promptHistoryActive,
    persistDraft: draftPersistence(options.sessionId, options.currentText),
  }
}

function draftPersistence(sessionId: string | null | undefined, text: string): PromptDraftPersistence | null {
  return sessionId
    ? { sessionId, text }
    : null
}

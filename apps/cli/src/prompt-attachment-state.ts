import type { PromptAttachmentKind } from "./prompt-attachments.js"

type PromptAttachmentTokenKind = "image" | "pdf" | "file"

export type PendingPromptAttachment = {
  id: string
  url: string
  mime: string
  filename: string
  kind: PromptAttachmentKind
  token: string
}

export type StoredPromptAttachment = Omit<PendingPromptAttachment, "token">

export function nextPromptAttachmentToken(
  attachments: readonly Pick<PendingPromptAttachment, "kind">[],
  kind: PromptAttachmentKind,
): string {
  const label = promptAttachmentTokenKind(kind)
  const count = attachments.filter((file) => promptAttachmentTokenKind(file.kind) === label).length + 1
  return `[${label} ${count}]`
}

export function filterPendingPromptAttachmentsForText(
  attachments: readonly PendingPromptAttachment[],
  value: string,
): PendingPromptAttachment[] {
  return attachments.filter((file) => value.includes(file.token))
}

export function addPendingPromptAttachments(
  current: readonly PendingPromptAttachment[],
  files: readonly StoredPromptAttachment[],
): {
  nextAttachments: PendingPromptAttachment[]
  addedAttachments: PendingPromptAttachment[]
} {
  const existing = new Set(current.map((file) => file.url))
  const nextFiles = files.filter((file) => !existing.has(file.url))
  if (nextFiles.length === 0) {
    return {
      nextAttachments: [...current],
      addedAttachments: [],
    }
  }
  const counts = {
    image: current.filter((file) => promptAttachmentTokenKind(file.kind) === "image").length,
    pdf: current.filter((file) => promptAttachmentTokenKind(file.kind) === "pdf").length,
    file: current.filter((file) => promptAttachmentTokenKind(file.kind) === "file").length,
  }
  const addedAttachments = nextFiles.map((file) => {
    const kind = promptAttachmentTokenKind(file.kind)
    counts[kind] += 1
    return { ...file, token: `[${kind} ${counts[kind]}]` }
  })
  return {
    nextAttachments: [...current, ...addedAttachments],
    addedAttachments,
  }
}

function promptAttachmentTokenKind(kind: PromptAttachmentKind): PromptAttachmentTokenKind {
  return kind === "image" ? "image" : kind === "pdf" ? "pdf" : "file"
}

export function removePendingPromptAttachmentToken(options: {
  attachments: readonly PendingPromptAttachment[]
  text: string
  token: string
}): {
  nextAttachments: PendingPromptAttachment[]
  nextText: string
  cursorOffset: number | null
  tokenFound: boolean
} {
  const nextAttachments = options.attachments.filter((file) => file.token !== options.token)
  const index = options.text.indexOf(options.token)
  if (index === -1) {
    return {
      nextAttachments,
      nextText: options.text,
      cursorOffset: null,
      tokenFound: false,
    }
  }
  let start = index
  let end = index + options.token.length
  if (start > 0 && options.text[start - 1] === " " && (end === options.text.length || options.text[end] === " " || options.text[end] === "\n")) {
    start -= 1
  } else if (end < options.text.length && options.text[end] === " ") {
    end += 1
  }
  return {
    nextAttachments,
    nextText: `${options.text.slice(0, start)}${options.text.slice(end)}`,
    cursorOffset: start,
    tokenFound: true,
  }
}

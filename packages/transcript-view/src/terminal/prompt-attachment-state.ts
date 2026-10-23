export type TerminalPromptAttachmentKind = "image" | "pdf" | "text" | "note" | "file"
export type TerminalPromptAttachmentStatus = "loading" | "ready" | "error"

export type TerminalPromptAttachmentView = {
  readonly id: string
  readonly filename: string
  readonly mime: string
  readonly size: number
  readonly kind: TerminalPromptAttachmentKind
  readonly previewUrl: string | null
}

export type TerminalPromptAttachment = TerminalPromptAttachmentView & {
  readonly sourceUrl: string
  contentsBase64: string | null
  status: TerminalPromptAttachmentStatus
  progress: number
  error: string | null
}

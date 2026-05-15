import { RGBA, SyntaxStyle } from "@opentui/core"

import type { PromptAttachmentKind } from "./prompt-attachments.js"

export type PromptAttachmentTokenKind = "image" | "pdf" | "file"

export const promptAttachmentTokenStyle = SyntaxStyle.create()

export const promptAttachmentTokenStyleIds: Record<PromptAttachmentTokenKind, number> = {
  image: promptAttachmentTokenStyle.registerStyle("prompt-token-image", {
    fg: RGBA.fromHex("#1f1400"),
    bg: RGBA.fromHex("#f0d77d"),
    bold: true,
  }),
  pdf: promptAttachmentTokenStyle.registerStyle("prompt-token-pdf", {
    fg: RGBA.fromHex("#09182b"),
    bg: RGBA.fromHex("#8cc0ff"),
    bold: true,
  }),
  file: promptAttachmentTokenStyle.registerStyle("prompt-token-file", {
    fg: RGBA.fromHex("#0d1f13"),
    bg: RGBA.fromHex("#8fd8a8"),
    bold: true,
  }),
}

export function promptAttachmentTokenKind(kind: PromptAttachmentKind): PromptAttachmentTokenKind {
  return kind === "image" ? "image" : kind === "pdf" ? "pdf" : "file"
}

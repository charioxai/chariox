import { readFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

import type { PromptAttachmentPart } from "./cli-types.js"

export type PromptAttachmentTransferOptions = {
  inlineLocalFiles: boolean
}

export async function preparePromptAttachmentsForSubmit(
  attachments: PromptAttachmentPart[],
  options: PromptAttachmentTransferOptions,
): Promise<PromptAttachmentPart[]> {
  if (!options.inlineLocalFiles || attachments.length === 0) {
    return attachments
  }
  return Promise.all(attachments.map(inlineAttachmentContents))
}

export function promptAttachmentTransferIsForced(): boolean {
  return process.env.CHARIOX_PROMPT_ATTACHMENT_TRANSFER === "1"
    || process.env.CHARIOX_NATIVE_TUI_FORCE_ATTACHMENT_TRANSFER === "1"
}

async function inlineAttachmentContents(attachment: PromptAttachmentPart): Promise<PromptAttachmentPart> {
  if (attachment.contents_base64) return attachment
  const localPath = localAttachmentPath(attachment.url)
  if (!localPath) return attachment
  const bytes = await readFile(localPath)
  return {
    ...attachment,
    contents_base64: bytes.toString("base64"),
  }
}

export function localAttachmentPath(url: string): string | null {
  if (path.isAbsolute(url)) return url
  try {
    const parsed = new URL(url)
    return parsed.protocol === "file:" ? fileURLToPath(parsed) : null
  } catch {
    return null
  }
}

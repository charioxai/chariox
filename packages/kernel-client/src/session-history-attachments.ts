import type { SessionHistoryPromptAttachment } from "./kernel-types.js"

export function cloneSessionHistoryPromptAttachments<T extends SessionHistoryPromptAttachment>(
  attachments: readonly T[],
): T[] {
  return attachments.map((attachment) => ({ ...attachment }))
}

export function mergeSessionHistoryPromptAttachments<T extends SessionHistoryPromptAttachment>(
  existing: readonly T[] | null | undefined,
  incoming: readonly T[] | null | undefined,
): T[] {
  const incomingAttachments = incoming ?? []
  if (!existing?.length) {
    return cloneSessionHistoryPromptAttachments(incomingAttachments)
  }
  if (!incomingAttachments.length) {
    return cloneSessionHistoryPromptAttachments(existing)
  }
  const existingByUrl = sessionHistoryPromptAttachmentsByUrl(existing)
  const incomingByUrl = sessionHistoryPromptAttachmentsByUrl(incomingAttachments)
  const sharedUrls = [...incomingByUrl.keys()].filter((url) => existingByUrl.has(url))
  if (sharedUrls.length === 0) {
    return cloneSessionHistoryPromptAttachments(
      sessionHistoryPromptAttachmentSetScore(incomingAttachments) > sessionHistoryPromptAttachmentSetScore(existing)
        ? incomingAttachments
        : existing,
    )
  }
  const base = existing.length >= incomingAttachments.length ? existing : incomingAttachments
  const alternateByUrl = base === existing ? incomingByUrl : existingByUrl
  const merged = base.map((attachment) => {
    const alternate = alternateByUrl.get(attachment.url)
    return { ...(alternate ? richerSessionHistoryPromptAttachment(attachment, alternate) : attachment) }
  })
  const mergedUrls = new Set(merged.map((attachment) => attachment.url))
  for (const attachment of base === existing ? incomingAttachments : existing) {
    if (!mergedUrls.has(attachment.url)) {
      merged.push({ ...attachment })
    }
  }
  return merged
}

function sessionHistoryPromptAttachmentsByUrl<T extends SessionHistoryPromptAttachment>(
  attachments: readonly T[],
): Map<string, T> {
  return new Map(attachments.map((attachment) => [attachment.url, attachment]))
}

function richerSessionHistoryPromptAttachment<T extends SessionHistoryPromptAttachment>(
  left: T,
  right: T,
): T {
  return sessionHistoryPromptAttachmentScore(right) > sessionHistoryPromptAttachmentScore(left) ? right : left
}

function sessionHistoryPromptAttachmentSetScore<T extends SessionHistoryPromptAttachment>(
  attachments: readonly T[],
): number {
  return attachments.reduce((sum, attachment) => sum + sessionHistoryPromptAttachmentScore(attachment), 0)
}

function sessionHistoryPromptAttachmentScore(attachment: SessionHistoryPromptAttachment): number {
  const previewScore = attachment.preview_url ? 10 : 0
  const nameScore = attachment.filename && attachment.filename !== "attachment" ? 1 : 0
  return 100 + previewScore + nameScore
}

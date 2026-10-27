import { mkdir, readFile, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import type { PromptAttachmentPart } from "../cli-types.js"
import { localAttachmentPath } from "../prompt-attachment-transfer.js"
import { classifyPromptAttachment } from "../prompt-attachments.js"

const CLAUDE_ATTACHMENT_CONTEXT_BYTES = 200_000

export type ClaudeNativePromptAttachmentReference = {
  start: number
  end: number
  attachment: PromptAttachmentPart
}

export function extractClaudeNativePromptAttachments(prompt: string, cwd: string): PromptAttachmentPart[] {
  return uniqueClaudeAttachmentReferences(
    extractClaudeNativePromptAttachmentReferences(prompt, cwd),
  ).map((reference) => reference.attachment)
}

export function extractClaudeNativePromptAttachmentReferences(
  prompt: string,
  cwd: string,
): ClaudeNativePromptAttachmentReference[] {
  const references: ClaudeNativePromptAttachmentReference[] = []
  for (const match of prompt.matchAll(/(?:^|\s)@(?:"([^"]+)"|'([^']+)'|([^\s]+))/g)) {
    const raw = match[1] ?? match[2] ?? match[3] ?? ""
    const candidate = trimAttachmentToken(raw)
    if (!candidate) continue
    const classified = classifyPromptAttachment(resolveClaudeAttachmentPath(candidate, cwd))
    if (!classified) continue
    const matched = match[0] ?? ""
    const leadingWhitespace = matched.startsWith("@") ? 0 : 1
    const start = (match.index ?? 0) + leadingWhitespace
    references.push({
      start,
      end: (match.index ?? 0) + matched.length,
      attachment: {
        url: classified.path,
        mime: classified.mime,
        filename: classified.filename,
      },
    })
  }
  return references
}

export function uniqueClaudeAttachmentReferences(
  references: ClaudeNativePromptAttachmentReference[],
): ClaudeNativePromptAttachmentReference[] {
  const byUrl = new Map<string, ClaudeNativePromptAttachmentReference>()
  for (const reference of references) {
    if (!byUrl.has(reference.attachment.url)) byUrl.set(reference.attachment.url, reference)
  }
  return Array.from(byUrl.values())
}

export function stripClaudeAttachmentMentions(
  prompt: string,
  references: ClaudeNativePromptAttachmentReference[],
): string {
  let cursor = 0
  let output = ""
  for (const reference of [...references].sort((left, right) => left.start - right.start)) {
    output += prompt.slice(cursor, reference.start)
    cursor = Math.max(cursor, reference.end)
  }
  output += prompt.slice(cursor)
  return output.replace(/\s{2,}/g, " ").trim()
}

export async function formatClaudeAttachmentContext(
  attachments: PromptAttachmentPart[],
  attachmentContextDir: string,
): Promise<string> {
  if (attachments.length === 0) return ""
  await mkdir(attachmentContextDir, { recursive: true })
  const blocks = await Promise.all(attachments.map((attachment, index) =>
    formatClaudeAttachmentBlock(attachment, index, attachmentContextDir),
  ))
  return [
    "The user included prompt attachments. Treat them as part of the current user request.",
    ...blocks,
  ].filter(Boolean).join("\n\n")
}

export async function formatClaudeNativeAttachmentPromptSuffix(
  attachments: PromptAttachmentPart[],
  attachmentContextDir: string,
): Promise<string> {
  if (attachments.length === 0) return ""
  await mkdir(attachmentContextDir, { recursive: true })
  const paths: string[] = []
  for (const [index, attachment] of attachments.entries()) {
    if (isClaudeTextAttachment(attachment)) continue
    const attachmentPath = await materializeClaudeAttachmentPath(attachment, index, attachmentContextDir)
    if (attachmentPath) paths.push(claudeAttachmentMention(attachmentPath))
  }
  return paths.join(" ")
}

export function joinClaudeVisiblePrompt(...parts: string[]): string {
  return parts.map((part) => part.trim()).filter(Boolean).join("\n\n")
}

export function joinClaudeAdditionalContext(...parts: string[]): string {
  return parts.map((part) => part.trim()).filter(Boolean).join("\n\n")
}

function trimAttachmentToken(value: string): string {
  return value.trim().replace(/[),.;:!?]+$/g, "")
}

function resolveClaudeAttachmentPath(value: string, cwd: string): string {
  if (value.startsWith("file://")) return fileURLToPath(value)
  if (value.startsWith("~/")) return path.join(homedir(), value.slice(2))
  return path.resolve(cwd, value)
}

function claudeAttachmentMention(filePath: string): string {
  if (!/[\s"'\\]/.test(filePath)) return `@${filePath}`
  return `@"${filePath.replace(/(["\\])/g, "\\$1")}"`
}

async function formatClaudeAttachmentBlock(
  attachment: PromptAttachmentPart,
  index: number,
  attachmentContextDir: string,
): Promise<string> {
  const displayName = attachment.filename || `attachment-${index + 1}`
  const attachmentPath = await materializeClaudeAttachmentPath(attachment, index, attachmentContextDir)
  const pieces = [
    `Attachment ${index + 1}: ${displayName}`,
    `MIME: ${attachment.mime}`,
    ...(attachmentPath ? [`Path: ${attachmentPath}`] : []),
  ]
  const text = await readClaudeTextAttachment(attachment, attachmentPath)
  if (text) {
    pieces.push("", "Content:", "```", text, "```")
  } else if (attachmentPath) {
    pieces.push("", "The attachment is available on disk at the path above.")
  } else {
    pieces.push("", "The attachment content is not available to the Claude native bridge.")
  }
  return pieces.join("\n")
}

async function materializeClaudeAttachmentPath(
  attachment: PromptAttachmentPart,
  index: number,
  attachmentContextDir: string,
): Promise<string | null> {
  const localPath = localAttachmentPath(attachment.url)
  if (localPath) return localPath
  if (!attachment.contents_base64) return null
  const filename = safeAttachmentFilename(attachment.filename, attachment.mime, index)
  const materialized = path.join(attachmentContextDir, filename)
  await writeFile(materialized, Buffer.from(attachment.contents_base64, "base64"))
  return materialized
}

async function readClaudeTextAttachment(
  attachment: PromptAttachmentPart,
  attachmentPath: string | null,
): Promise<string | null> {
  const bytes = attachment.contents_base64
    ? Buffer.from(attachment.contents_base64, "base64")
    : attachmentPath && isClaudeTextAttachment(attachment)
      ? await readFile(attachmentPath).catch(() => null)
      : null
  if (!bytes || bytes.length > CLAUDE_ATTACHMENT_CONTEXT_BYTES || !isClaudeTextAttachment(attachment)) {
    return null
  }
  return bytes.toString("utf8")
}

function isClaudeTextAttachment(attachment: PromptAttachmentPart): boolean {
  if (attachment.mime.startsWith("text/")) return true
  if (attachment.mime === "application/json" || attachment.mime.endsWith("+json")) return true
  const filename = attachment.filename?.toLowerCase() ?? ""
  return /\.(md|txt|json|jsonl|csv|ts|tsx|js|jsx|mjs|py|rs|go|java|rb|sh|zsh|yaml|yml|toml|xml|html|css|scss|sql|log)$/.test(filename)
}

function safeAttachmentFilename(filename: string | null | undefined, mime: string, index: number): string {
  const fallback = `attachment-${index + 1}${extensionForMime(mime)}`
  const base = path.basename(filename || fallback).replace(/[^A-Za-z0-9._-]/g, "_")
  return `${index + 1}-${base || fallback}`
}

function extensionForMime(mime: string): string {
  if (mime === "image/png") return ".png"
  if (mime === "image/jpeg") return ".jpg"
  if (mime === "image/gif") return ".gif"
  if (mime === "image/webp") return ".webp"
  if (mime === "application/pdf") return ".pdf"
  if (mime === "application/json") return ".json"
  if (mime.startsWith("text/")) return ".txt"
  return ".bin"
}

import path from "node:path"

import type { PromptAttachmentPart } from "../cli-types.js"

export function extractCodexPrompt(params: Record<string, unknown> | undefined): string {
  const input = Array.isArray(params?.input) ? params.input : []
  const text = input.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  }).join("\n")
  return text.endsWith("\n") ? text : `${text}\n`
}

// MP-08 / MP-10: Codex clears a pending steer by comparing its committed
// user-message text with the submitted input. Preserve that text exactly;
// the terminal newline used for initial prompts would strand the receipt.
export function extractCodexSteeringPrompt(params: Record<string, unknown> | undefined): string {
  const input = Array.isArray(params?.input) ? params.input : []
  return input.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  }).join("")
}

export function extractCodexAttachments(params: Record<string, unknown> | undefined): PromptAttachmentPart[] {
  const input = Array.isArray(params?.input) ? params.input : []
  return input.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    if (record.type === "image" && typeof record.url === "string") {
      return [{
        url: record.url,
        mime: inferImageMime(record.url),
        filename: filenameFromUrl(record.url),
      }]
    }
    if (record.type === "localImage" && typeof record.path === "string") {
      return [{
        url: record.path,
        mime: inferImageMime(record.path),
        filename: path.basename(record.path),
      }]
    }
    return []
  })
}

function inferImageMime(value: string): string {
  const lower = value.toLowerCase()
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg"
  if (lower.endsWith(".gif")) return "image/gif"
  if (lower.endsWith(".webp")) return "image/webp"
  return "image/png"
}

function filenameFromUrl(value: string): string | null {
  try {
    const url = new URL(value)
    const name = path.basename(url.pathname)
    return name || null
  } catch {
    const name = path.basename(value)
    return name || null
  }
}

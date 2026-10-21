import path from "node:path"

import type { PromptAttachmentPart } from "../cli-types.js"

export function extractCodexPrompt(params: Record<string, unknown> | undefined): string {
  const input = Array.isArray(params?.input) ? params.input : []
  const text = input.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  }).join("\n")
  // Skill input paths come from an untrusted native client. Preserve only the
  // invocation label; the kernel resolves a current source-bound selection.
  const missing = extractCodexSkillNames(params).filter(name =>
    !new RegExp(`\\$${name}(?![a-zA-Z0-9_-])`).test(text))
  const prompt = [text, ...missing.map(name => `$${name}`)].filter(Boolean).join("\n")
  return prompt.endsWith("\n") ? prompt : `${prompt}\n`
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

// MP-08 / MP-10 / MP-11: never forward a client-supplied native skill path.
export function extractCodexSkillNames(params: Record<string, unknown> | undefined): string[] {
  const names = new Set<string>()
  for (const part of Array.isArray(params?.input) ? params.input : []) {
    if (!part || typeof part !== "object" || part.type !== "skill") continue
    if (typeof part.name !== "string" || !/^[a-zA-Z0-9_-]{1,128}$/.test(part.name)) {
      throw new Error("invalid Codex skill input; refresh skills and select again")
    }
    names.add(part.name)
  }
  return [...names]
}

// A skill item whose path is not a Chariox projection handle came from the
// upstream native completion. The path is discarded; only that choice survives
// and the kernel still validates the source-bound selection it maps to.
export function extractCodexNativeSkillNames(params: Record<string, unknown> | undefined): Set<string> {
  const names = new Set<string>()
  for (const part of Array.isArray(params?.input) ? params.input : []) {
    if (part?.type === "skill" && typeof part.name === "string" && typeof part.path === "string"
      && !part.path.startsWith("chariox-skill://")) names.add(part.name)
  }
  return names
}

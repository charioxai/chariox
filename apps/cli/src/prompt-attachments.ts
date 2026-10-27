import fs from "node:fs"
import path from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"

export type PromptAttachmentKind = "image" | "pdf" | "text"

export type ParsedPromptAttachment = {
  path: string
  filename: string
  mime: string
  kind: PromptAttachmentKind
}

type PromptAttachmentEditAction = "backspace" | "delete"

type PromptAttachmentEditSelection = {
  start: number
  end: number
}

type PromptAttachmentTokenSpan = {
  token: string
  start: number
  end: number
}

type PromptAttachmentRemovalEdit = {
  kind: "remove-attachments"
  start: number
  end: number
  tokens: string[]
}

type PromptAttachmentTextEdit = {
  kind: "delete-text"
  start: number
  end: number
}

type PromptAttachmentNoopEdit = {
  kind: "noop"
}

export type ResolvedPromptAttachmentEdit = PromptAttachmentRemovalEdit | PromptAttachmentTextEdit | PromptAttachmentNoopEdit

const IMAGE_MIME = new Map<string, string>([
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".gif", "image/gif"],
  [".webp", "image/webp"],
  [".bmp", "image/bmp"],
  [".svg", "image/svg+xml"],
])

const TEXT_EXT = new Set([
  ".c",
  ".cc",
  ".conf",
  ".cpp",
  ".css",
  ".csv",
  ".go",
  ".h",
  ".hpp",
  ".html",
  ".ini",
  ".java",
  ".js",
  ".json",
  ".jsx",
  ".log",
  ".md",
  ".mjs",
  ".py",
  ".rb",
  ".rs",
  ".scss",
  ".sh",
  ".sql",
  ".svg",
  ".toml",
  ".ts",
  ".tsx",
  ".txt",
  ".xml",
  ".yaml",
  ".yml",
  ".zsh",
])

const TEXT_NAME = new Set([
  "dockerfile",
  "gitignore",
  "makefile",
  "readme",
  "readme.md",
  "license",
])

export function extractDroppedPromptAttachments(previous: string, next: string, cwd: string) {
  if (!next || next === previous || next.length <= previous.length) {
    return null
  }
  let start = 0
  while (start < previous.length && start < next.length && previous[start] === next[start]) {
    start += 1
  }
  let previousEnd = previous.length - 1
  let nextEnd = next.length - 1
  while (previousEnd >= start && nextEnd >= start && previous[previousEnd] === next[nextEnd]) {
    previousEnd -= 1
    nextEnd -= 1
  }
  const added = next.slice(start, nextEnd + 1)
  const files = parsePromptAttachmentPaths(added, cwd)
  if (files.length === 0) {
    return null
  }
  return {
    nextText: previous,
    insertAt: start,
    files,
  }
}

export function parsePromptAttachmentCommand(value: string, cwd: string) {
  const files = parsePromptAttachmentPaths(value, cwd)
  if (files.length === 0) {
    return null
  }
  return files
}

export function parsePromptAttachmentPaths(value: string, cwd: string) {
  const words = splitShellWords(value.trim())
  if (words.length === 0) {
    return []
  }
  const files = words
    .map((word) => classifyPromptAttachment(resolvePromptAttachmentPath(word, cwd)))
    .filter((file): file is ParsedPromptAttachment => file !== null)
  return files.length === words.length ? dedupePromptAttachments(files) : []
}

export function formatPromptAttachmentSummary(
  files: Array<Pick<ParsedPromptAttachment, "filename" | "kind">>,
) {
  if (files.length === 0) {
    return ""
  }
  return files
    .map((file) => `[${attachmentLabel(file.kind)} ${file.filename}]`)
    .join(" ")
}

export function attachmentLabel(kind: PromptAttachmentKind) {
  if (kind === "image") {
    return "IMG"
  }
  if (kind === "pdf") {
    return "PDF"
  }
  return "TXT"
}

export function resolvePromptAttachmentEdit(
  text: string,
  tokens: string[],
  action: PromptAttachmentEditAction,
  cursor: number,
  selection?: PromptAttachmentEditSelection | null,
): ResolvedPromptAttachmentEdit | null {
  const normalizedSelection = selection && selection.start !== selection.end
    ? {
        start: Math.min(selection.start, selection.end),
        end: Math.max(selection.start, selection.end),
      }
    : null
  const spans = collectPromptAttachmentTokenSpans(text, tokens)
  if (normalizedSelection) {
    const matches = spans.filter((span) => span.start < normalizedSelection.end && span.end > normalizedSelection.start)
    if (matches.length === 0) {
      return {
        kind: "delete-text",
        start: normalizedSelection.start,
        end: normalizedSelection.end,
      }
    }
    return {
      kind: "remove-attachments",
      start: Math.min(normalizedSelection.start, ...matches.map((span) => span.start)),
      end: Math.max(normalizedSelection.end, ...matches.map((span) => span.end)),
      tokens: matches.map((span) => span.token),
    }
  }
  if (action === "backspace") {
    const deleteIndex = cursor - 1
    if (deleteIndex < 0 || deleteIndex >= text.length) {
      return null
    }
    const target = text[deleteIndex]
    if (!target || /\s/.test(target)) {
      return null
    }
    const matches = spans.filter((span) => deleteIndex >= span.start && deleteIndex < span.end)
    if (matches.length === 0) {
      return null
    }
    return {
      kind: "remove-attachments",
      start: Math.min(...matches.map((span) => span.start)),
      end: Math.max(...matches.map((span) => span.end)),
      tokens: matches.map((span) => span.token),
    }
  }
  const boundaryMatch = spans.find((span) => cursor === span.end - 1)
  if (boundaryMatch) {
    const nextIndex = boundaryMatch.end
    if (nextIndex >= text.length) {
      return { kind: "noop" }
    }
    const nextSpan = spans.find((span) => nextIndex >= span.start && nextIndex < span.end)
    if (nextSpan) {
      return {
        kind: "remove-attachments",
        start: nextSpan.start,
        end: nextSpan.end,
        tokens: [nextSpan.token],
      }
    }
    return {
      kind: "delete-text",
      start: nextIndex,
      end: nextIndex + 1,
    }
  }
  if (cursor < 0 || cursor >= text.length) {
    return null
  }
  const target = text[cursor]
  if (!target || /\s/.test(target)) {
    return null
  }
  const matches = spans.filter((span) => cursor >= span.start && cursor < span.end - 1)
  if (matches.length === 0) {
    return null
  }
  return {
    kind: "remove-attachments",
    start: Math.min(...matches.map((span) => span.start)),
    end: Math.max(...matches.map((span) => span.end)),
    tokens: matches.map((span) => span.token),
  }
}

export function classifyPromptAttachment(filePath: string) {
  let stat: fs.Stats
  try {
    stat = fs.statSync(filePath)
  } catch {
    return null
  }
  if (!stat.isFile()) {
    return null
  }
  const filename = path.basename(filePath)
  const extension = path.extname(filename).toLowerCase()
  const image = IMAGE_MIME.get(extension)
  if (image) {
    return {
      path: filePath,
      filename,
      mime: image,
      kind: "image" as const,
    }
  }
  if (extension === ".pdf") {
    return {
      path: filePath,
      filename,
      mime: "application/pdf",
      kind: "pdf" as const,
    }
  }
  if (TEXT_EXT.has(extension) || TEXT_NAME.has(filename.toLowerCase())) {
    return {
      path: filePath,
      filename,
      mime: extension === ".svg" ? "image/svg+xml" : "text/plain",
      kind: extension === ".svg" ? "image" : "text",
    }
  }
  return null
}

function dedupePromptAttachments(files: ParsedPromptAttachment[]) {
  const seen = new Set<string>()
  return files.filter((file) => {
    if (seen.has(file.path)) {
      return false
    }
    seen.add(file.path)
    return true
  })
}

function collectPromptAttachmentTokenSpans(text: string, tokens: string[]) {
  return tokens
    .map((token) => {
      const start = text.indexOf(token)
      return start === -1 ? null : { token, start, end: start + token.length }
    })
    .filter((span): span is PromptAttachmentTokenSpan => span !== null)
}

function resolvePromptAttachmentPath(value: string, cwd: string) {
  if (value.startsWith("file://")) {
    return fileURLToPath(value)
  }
  if (value.startsWith("~/")) {
    return path.join(homedir(), value.slice(2))
  }
  return path.resolve(cwd, value)
}

function splitShellWords(value: string) {
  const words: string[] = []
  let current = ""
  let quote: '"' | "'" | null = null
  let escaped = false
  for (const char of value) {
    if (escaped) {
      current += char
      escaped = false
      continue
    }
    if (char === "\\") {
      escaped = true
      continue
    }
    if (quote) {
      if (char === quote) {
        quote = null
        continue
      }
      current += char
      continue
    }
    if (char === '"' || char === "'") {
      quote = char
      continue
    }
    if (/\s/.test(char)) {
      if (current) {
        words.push(current)
        current = ""
      }
      continue
    }
    current += char
  }
  if (escaped) {
    current += "\\"
  }
  if (current) {
    words.push(current)
  }
  return words
}

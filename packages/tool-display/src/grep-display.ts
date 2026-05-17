import {
  displayGrepPath,
  renderPathCodeBlockCollection,
} from "./code-blocks.js"
import { formatToolStatusBadge } from "./status.js"
import {
  nonEmpty,
  trimTrailingNewlines,
} from "./strings.js"
import type {
  ToolTranscriptUpdate,
} from "./types.js"

type GrepInput = {
  pattern?: unknown
  path?: unknown
  include?: unknown
}

export function formatGrepTranscriptUpdate(update: ToolTranscriptUpdate) {
  if (update.tool !== "grep") {
    return null
  }

  const input = readGrepInput(update.input)
  const output = typeof update.output === "string" ? trimTrailingNewlines(update.output) : null
  if (!input || !output) {
    return null
  }

  const parsed = parseGrepOutput(output, input)
  if (!parsed) {
    return null
  }

  return [
    `**grep**${formatToolStatusBadge(nonEmpty(update.status))}`,
    `Pattern: \`${input.pattern}\`${parsed.summary}`,
    ...parsed.blocks,
  ].join("\n")
}

function readGrepInput(input: unknown) {
  if (!input || typeof input !== "object") {
    return null
  }

  const value = input as GrepInput
  const pattern = typeof value.pattern === "string" ? value.pattern.trim() : ""
  if (!pattern) {
    return null
  }

  const result: { pattern: string; path?: string; include?: string } = { pattern }
  if (typeof value.path === "string" && value.path.trim()) {
    result.path = value.path.trim()
  }
  if (typeof value.include === "string" && value.include.trim()) {
    result.include = value.include.trim()
  }
  return result
}

function parseGrepOutput(output: string, input: { path?: string; include?: string }) {
  const lines = output.split(/\r?\n/)
  const summaryLine = lines[0]?.trim() ?? ""
  if (/^No files found\.?$/i.test(summaryLine)) {
    return {
      summary: formatGrepSearchScope(input, 0),
      blocks: ["```text\nNo files found\n```"],
    }
  }
  const matchCount = Number(/^Found (\d+) matches?/.exec(summaryLine)?.[1] ?? "0")
  const files = new Map<string, string[]>()
  let currentFile: string | null = null

  for (const line of lines.slice(1)) {
    if (!line.trim()) {
      continue
    }
    if (!line.startsWith("  ") && line.endsWith(":")) {
      currentFile = line.slice(0, -1)
      files.set(currentFile, [])
      continue
    }
    if (currentFile) {
      files.get(currentFile)?.push(line.trim())
    }
  }

  if (files.size === 0) {
    return null
  }

  const entries = [...files.entries()]
  const totalMatches = matchCount > 0 ? matchCount : entries.reduce((sum, [, fileLines]) => sum + fileLines.length, 0)
  const summary = entries.length === 1
    ? ` in ${displayGrepPath(entries[0]![0], input.path)} (${totalMatches} matches)`
    : ` (${totalMatches} matches in ${entries.length} files)`
  const blocks = renderPathCodeBlockCollection(
    entries.map(([filePath, fileLines]) => ({
      filePath,
      content: fileLines.join("\n"),
      rootPath: input.path,
    })),
  )

  return { summary, blocks }
}

function formatGrepSearchScope(input: { path?: string; include?: string }, matches: number) {
  const location = input.path ? ` in ${displayGrepPath(input.path)}` : ""
  const include = input.include ? ` [${input.include}]` : ""
  return `${location}${include} (${matches} matches)`
}

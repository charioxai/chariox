import { guessPathFenceLanguage } from "./language.js"
import { trimTrailingNewlines } from "./strings.js"

const TOOL_BLOB_VISIBLE_LINES = 10

export type EmbeddedFileBlock = {
  filePath: string
  content: string
  rootPath?: string
}

type PathCodeBlock = {
  filePath: string
  content: string
  rootPath?: string | undefined
}

export function renderLabeledCodeBlock(label: string, content: string, language = "text") {
  const body = truncateToolBlob(content)
  const fence = codeFence(body)
  return `**${label}**\n${fence}${language}\n${body}\n${fence}`
}

export function renderPathCodeBlock(filePath: string, content: string, rootPath?: string) {
  const body = truncateToolBlob(content)
  const fence = codeFence(body)
  return [
    `\`${displayGrepPath(filePath, rootPath)}\``,
    `${fence}${guessPathFenceLanguage(filePath)}\n${body}\n${fence}`,
  ].join("\n")
}

export function codeFence(content: string) {
  const matches = content.match(/`+/g) ?? []
  const width = matches.reduce((max, value) => Math.max(max, value.length), 2) + 1
  return "`".repeat(width)
}

export function truncateToolBlob(text: string) {
  return collapseMiddleLines(text, TOOL_BLOB_VISIBLE_LINES)
}

export function collapseMiddleLines(text: string, visibleLines: number) {
  const lines = text.split(/\r?\n/)
  if (lines.length <= visibleLines * 2 + 1) {
    return trimTrailingNewlines(text)
  }

  const headCount = visibleLines
  const tailCount = visibleLines
  return [...lines.slice(0, headCount), "...", ...lines.slice(-tailCount)].join("\n")
}

export function displayGrepPath(filePath: string, rootPath?: string) {
  if (rootPath && filePath.startsWith(`${rootPath}/`)) {
    return filePath.slice(rootPath.length + 1)
  }
  return filePath
}

export function renderPathCodeBlockCollection(items: PathCodeBlock[]) {
  if (items.length <= 1) {
    return items.map((item) => renderPathCodeBlock(item.filePath, item.content, item.rootPath))
  }

  const totalLines = items.reduce((sum, item) => sum + countPathCodeBlockLines(item), 0)
  if (totalLines <= TOOL_BLOB_VISIBLE_LINES * 2 + 1) {
    return items.map((item) => renderPathCodeBlock(item.filePath, item.content, item.rootPath))
  }

  const head = takePathCodeBlockSide(items, TOOL_BLOB_VISIBLE_LINES, "head")
  const tail = takePathCodeBlockSide(items, TOOL_BLOB_VISIBLE_LINES, "tail")
  return [...head, "...", ...tail]
}

export function parseEmbeddedFileBlock(value: string): EmbeddedFileBlock | null {
  const pathMatch = value.match(/<path>([\s\S]*?)<\/path>/)
  const contentMatch = value.match(/<content>([\s\S]*?)<\/content>/)
  if (!pathMatch || !contentMatch) {
    return null
  }

  const filePath = trimTrailingNewlines(pathMatch[1] ?? "").trim()
  if (!filePath) {
    return null
  }

  return {
    filePath,
    content: trimTrailingNewlines(contentMatch[1] ?? ""),
  }
}

function takePathCodeBlockSide(items: PathCodeBlock[], budget: number, side: "head" | "tail") {
  const ordered = side === "head" ? items : [...items].reverse()
  const blocks: string[] = []
  let remaining = budget

  for (const item of ordered) {
    if (remaining <= 0) {
      break
    }
    const lineCount = countPathCodeBlockLines(item)
    if (lineCount <= remaining) {
      blocks.push(renderPathCodeBlock(item.filePath, item.content, item.rootPath))
      remaining -= lineCount
      continue
    }
    blocks.push(renderPartialPathCodeBlock(item, remaining, side))
    remaining = 0
  }

  return side === "head" ? blocks : blocks.reverse()
}

function countPathCodeBlockLines(item: PathCodeBlock) {
  const body = truncateToolBlob(item.content)
  return 3 + body.split(/\r?\n/).length
}

function renderPartialPathCodeBlock(item: PathCodeBlock, budget: number, side: "head" | "tail") {
  if (budget <= 1) {
    return `\`${displayGrepPath(item.filePath, item.rootPath)}\``
  }

  const language = guessPathFenceLanguage(item.filePath)
  const lines = truncateToolBlob(item.content).split(/\r?\n/)
  const visible = Math.max(1, budget - 3)
  const clipped = side === "head" ? lines.slice(0, visible) : lines.slice(-visible)
  const body = clipped.join("\n")
  const fence = codeFence(body)
  return [
    `\`${displayGrepPath(item.filePath, item.rootPath)}\``,
    `${fence}${language}`,
    body,
    fence,
  ].join("\n")
}

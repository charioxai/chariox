import MarkdownIt from "markdown-it"
import { normalizeMarkdownFenceInfoStrings } from "./transcript-markdown.js"
import { renderTranscriptMath } from "./transcript-math.js"

// MP-08 / MP-10 (PANE-09): terminal equivalents for rich Markdown that
// OpenTUI's Markdown renderable shows as raw source. Transforms run on prose
// only: fenced code blocks and inline code spans pass through byte-for-byte.

const markdown = new MarkdownIt({ html: false })
const taskMarkerPattern = /^(\s*(?:[-*+]|\d{1,9}[.)])[ \t]+)\[([ xX])\](?=[ \t])/gm
const imagePattern = /!\[([^\]\n]*)\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"\n]*")?\s*\)/g
const dataImagePattern = /^data:image\/([a-z0-9.+-]+);base64,([a-z0-9+/=]+)$/i

export function prepareTranscriptMarkdown(text: string): string {
  return mapMarkdownProse(normalizeMarkdownFenceInfoStrings(text), (prose) => (
    mapOutsideCodeSpans(prose, (text) => renderTranscriptMath(renderImages(renderTaskMarkers(text))))
  ))
}

// Terminals get a one-line placeholder: alt text, file name and host (or
// inline size), and for remote images a clickable OSC-8 link to open it.
function renderImages(prose: string): string {
  return prose.replace(imagePattern, (_match, rawAlt: string, src: string) => {
    const alt = escapeLinkText(rawAlt.trim() || "image")
    const inline = dataImagePattern.exec(src)
    if (inline) return `▣ image: ${alt} · ${inline[1]!.toLowerCase()} ${formatBytes(base64Bytes(inline[2]!))} (inline; open in the web app)`
    if (/^https?:\/\//i.test(src)) {
      const url = safeUrl(src)
      if (!url) return `▣ image: ${alt}`
      const file = decodeSafe(url.pathname.split("/").pop() ?? "")
      return `[▣ image: ${alt} · ${escapeLinkText([file, url.host].filter(Boolean).join(" · "))} · size unknown (open link)](${url.href})`
    }
    return `▣ image: ${alt} · ${escapeLinkText(src.replace(/^file:\/\//i, ""))} · size unknown (open in the web app)`
  })
}

function renderTaskMarkers(prose: string): string {
  return prose.replace(taskMarkerPattern, (_match, prefix: string, mark: string) => `${prefix}${mark === " " ? "☐" : "☑"}`)
}

// Applies `transform` to every run of lines outside fenced code blocks. An
// unterminated fence (still streaming) protects everything after it.
export function mapMarkdownProse(text: string, transform: (prose: string) => string): string {
  const lines = text.split("\n")
  const protectedLines = new Set<number>()
  // Token maps retain source line numbers inside lists and blockquotes too.
  // Use the Markdown parser rather than guessing fences from indentation.
  for (const token of markdown.parse(text, {})) {
    if ((token.type === "fence" || token.type === "code_block") && token.map) {
      for (let line = token.map[0]; line < token.map[1]; line += 1) protectedLines.add(line)
    }
  }
  const output: string[] = []
  let prose: string[] = []
  const flush = () => {
    if (prose.length > 0) output.push(transform(prose.join("\n")))
    prose = []
  }
  for (const [index, line] of lines.entries()) {
    if (protectedLines.has(index)) { flush(); output.push(line) }
    else prose.push(line)
  }
  flush()
  return output.join("\n")
}

// Applies `transform` to the text between inline code spans (CommonMark
// backtick runs of equal length); unmatched backticks are ordinary text.
export function mapOutsideCodeSpans(text: string, transform: (text: string) => string): string {
  let output = ""
  let plainStart = 0
  let index = 0
  while (index < text.length) {
    if (text[index] !== "`") {
      index += 1
      continue
    }
    let runEnd = index
    while (text[runEnd] === "`") runEnd += 1
    const run = text.slice(index, runEnd)
    const close = findClosingRun(text, runEnd, run.length)
    if (close < 0) {
      index = runEnd
      continue
    }
    output += transform(text.slice(plainStart, index)) + text.slice(index, close + run.length)
    index = close + run.length
    plainStart = index
  }
  return output + transform(text.slice(plainStart))
}

function findClosingRun(text: string, from: number, length: number): number {
  let index = from
  while (index < text.length) {
    const start = text.indexOf("`", index)
    if (start < 0) return -1
    let end = start
    while (text[end] === "`") end += 1
    if (end - start === length) return start
    index = end
  }
  return -1
}

function escapeLinkText(value: string): string {
  if (!/[\[\]\\`*_<>~]/.test(value)) return value
  // A code span keeps filenames/alt labels literal without visible escapes
  // or extra Markdown token blocks around escaped brackets.
  const runs = [...value.matchAll(/`+/g)].map(match => match[0].length)
  const fence = "`".repeat(Math.max(0, ...runs) + 1)
  return `${fence} ${value} ${fence}`
}

function safeUrl(value: string): URL | null {
  try {
    return new URL(value)
  } catch {
    return null
  }
}

function decodeSafe(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

function base64Bytes(data: string): number {
  return Math.floor(data.length * 3 / 4) - (data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0)
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

import { normalizeMarkdownFenceInfoStrings } from "./transcript-markdown.js"

// MP-08 / MP-10 (PANE-09): terminal equivalents for rich Markdown that
// OpenTUI's Markdown renderable shows as raw source. Transforms run on prose
// only: fenced code blocks and inline code spans pass through byte-for-byte.

const fenceOpenPattern = /^ {0,3}(`{3,}|~{3,})/
const taskMarkerPattern = /^(\s*(?:[-*+]|\d{1,9}[.)])[ \t]+)\[([ xX])\](?=[ \t])/gm
const imagePattern = /!\[([^\]\n]*)\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"\n]*")?\s*\)/g
const dataImagePattern = /^data:image\/([a-z0-9.+-]+);base64,([a-z0-9+/=]+)$/i

export function prepareTranscriptMarkdown(text: string): string {
  return mapMarkdownProse(normalizeMarkdownFenceInfoStrings(text), (prose) => (
    mapOutsideCodeSpans(prose, (text) => renderImages(renderTaskMarkers(text)))
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
      return `[▣ image: ${alt} · ${escapeLinkText([file, url.host].filter(Boolean).join(" · "))}](${url.href})`
    }
    return `▣ image: ${alt} · ${escapeLinkText(src.replace(/^file:\/\//i, ""))} (workspace file)`
  })
}

function renderTaskMarkers(prose: string): string {
  return prose.replace(taskMarkerPattern, (_match, prefix: string, mark: string) => `${prefix}${mark === " " ? "☐" : "☑"}`)
}

// Applies `transform` to every run of lines outside fenced code blocks. An
// unterminated fence (still streaming) protects everything after it.
export function mapMarkdownProse(text: string, transform: (prose: string) => string): string {
  const lines = text.split("\n")
  const output: string[] = []
  let prose: string[] = []
  let fence: { readonly marker: string; readonly length: number } | null = null
  const flush = () => {
    if (prose.length > 0) output.push(transform(prose.join("\n")))
    prose = []
  }
  for (const line of lines) {
    if (fence) {
      output.push(line)
      const close = fenceOpenPattern.exec(line)
      if (close && close[1]![0] === fence.marker && close[1]!.length >= fence.length && line.trim() === close[1]) fence = null
      continue
    }
    const open = fenceOpenPattern.exec(line)
    if (open && !(open[1]![0] === "`" && line.slice(line.indexOf(open[1]!) + open[1]!.length).includes("`"))) {
      flush()
      output.push(line)
      fence = { marker: open[1]![0]!, length: open[1]!.length }
      continue
    }
    prose.push(line)
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
  return value.replace(/[[\]\\]/g, "\\$&")
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

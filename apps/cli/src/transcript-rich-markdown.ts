import { normalizeMarkdownFenceInfoStrings } from "./transcript-markdown.js"

// MP-08 / MP-10 (PANE-09): terminal equivalents for rich Markdown that
// OpenTUI's Markdown renderable shows as raw source. Transforms run on prose
// only: fenced code blocks and inline code spans pass through byte-for-byte.

const fenceOpenPattern = /^ {0,3}(`{3,}|~{3,})/
const taskMarkerPattern = /^(\s*(?:[-*+]|\d{1,9}[.)])[ \t]+)\[([ xX])\](?=[ \t])/gm

export function prepareTranscriptMarkdown(text: string): string {
  return mapMarkdownProse(normalizeMarkdownFenceInfoStrings(text), (prose) => (
    mapOutsideCodeSpans(prose, renderTaskMarkers)
  ))
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

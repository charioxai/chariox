import { CodeRenderable, type MarkdownOptions, type TextChunk } from "@opentui/core"
import { providerLoginUrl } from "./provider-login-link.js"

// MP-08/MP-10/MP-11: native links retain visible text and styling, including
// bare Markdown URLs that OpenTUI's syntax-based link detection misses.
export function linkifyTranscriptChunks(chunks: TextChunk[]): TextChunk[] {
  const value = chunks.map(chunk => chunk.text).join("")
  const ranges = [...value.matchAll(/https?:\/\/[^\s<>\u0000-\u001f\u007f]+/gu)].flatMap(match => {
    const start = match.index!
    const follows = value.slice(start + match[0].length).split("\n")[0]!.trim()
    const url = follows ? match[0].replace(/[.,;:]+$/, "") : match[0]
    return providerLoginUrl(url) ? [{ start, end: start + url.length, url }] : []
  })
  let offset = 0
  return chunks.flatMap(chunk => {
    const end = offset + chunk.text.length
    const pieces: TextChunk[] = []
    let start = offset
    for (const range of ranges) {
      if (range.end <= start || range.start >= end) continue
      const from = Math.max(start, range.start)
      const to = Math.min(end, range.end)
      if (from > start) pieces.push({ ...chunk, text: chunk.text.slice(start - offset, from - offset) })
      pieces.push({ ...chunk, text: chunk.text.slice(from - offset, to - offset), link: chunk.link ?? { url: range.url } })
      start = to
    }
    if (start < end) pieces.push({ ...chunk, text: chunk.text.slice(start - offset) })
    offset = end
    return pieces
  })
}

export const renderTranscriptMarkdownNode: NonNullable<MarkdownOptions["renderNode"]> = (_token, context) => {
  const node = context.defaultRender()
  if (node instanceof CodeRenderable) {
    const original = node.onChunks
    node.onChunks = async (chunks, details) => linkifyTranscriptChunks(await original?.(chunks, details) ?? chunks)
  }
  return node
}

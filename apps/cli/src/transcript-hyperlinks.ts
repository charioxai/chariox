import { BoxRenderable, CodeRenderable, StyledText, type MarkdownOptions, type TextBuffer, type TextChunk } from "@opentui/core"
import { parseUnifiedDiff } from "@chariox/tool-display"
import { isDiffFiletype, TranscriptDiffRenderable } from "./transcript-diff-render.js"
import { providerLoginUrl } from "./provider-login-link.js"

// MP-08/MP-10/MP-11: native links retain visible text and styling, including
// bare Markdown URLs that OpenTUI's syntax-based link detection misses.
export function linkifyTranscriptChunks(chunks: TextChunk[]): TextChunk[] {
  const value = chunks.map(chunk => chunk.text).join("")
  const ranges = [...value.matchAll(/https?:\/\/[^\s<>\u0000-\u001f\u007f]+/gu)].flatMap(match => {
    const start = match.index!
    const follows = value.slice(start + match[0].length).split("\n")[0]!.trim()
    const precedes = value.slice(value.lastIndexOf("\n", start - 1) + 1, start).trim()
    const url = precedes || follows ? match[0].replace(/[.,;:]+$/, "") : match[0]
    return providerLoginUrl(url) ? [{ start, end: start + url.length, url }] : []
  })
  let offset = 0
  return chunks.flatMap(original => {
    // MP-08 / MP-11: syntax detection is not URL admission. Preserve allowed
    // descriptive links, and make visible URL text link to its own target.
    const { link, ...plain } = original
    const chunk = link && providerLoginUrl(link.url) ? original : plain
    const end = offset + chunk.text.length
    const pieces: TextChunk[] = []
    let start = offset
    for (const range of ranges) {
      if (range.end <= start || range.start >= end) continue
      const from = Math.max(start, range.start)
      const to = Math.min(end, range.end)
      if (from > start) pieces.push({ ...chunk, text: chunk.text.slice(start - offset, from - offset) })
      pieces.push({ ...chunk, text: chunk.text.slice(from - offset, to - offset), link: { url: range.url } })
      start = to
    }
    if (start < end) pieces.push({ ...chunk, text: chunk.text.slice(start - offset) })
    offset = end
    return pieces
  })
}

function linkableCodeFiletype(value: string | undefined): string {
  return !value || /^(?:text|txt|plaintext)$/i.test(value) ? "markdown" : value
}

class TranscriptCodeRenderable extends CodeRenderable {
  override get filetype(): string | undefined { return super.filetype }
  override set filetype(value: string | undefined) { super.filetype = linkableCodeFiletype(value) }
}

// MP-08/MP-10/MP-11: CodeRenderable paints plain fallback text before its
// asynchronous syntax worker finishes. Keep those already-visible URLs
// clickable during streaming, resize and worker delays as well.
function linkifyTranscriptFallback(node: CodeRenderable): void {
  const buffer = (node as unknown as {textBuffer:TextBuffer}).textBuffer
  const setText = buffer.setText
  buffer.setText = function (value: string) {
    const chunks = linkifyTranscriptChunks([{__isChunk:true,text:value}])
    if (chunks.some(chunk => chunk.link)) this.setStyledText(new StyledText(chunks))
    else setText.call(this, value)
  }
  // The constructor may already have painted plain content before this hook.
  // Link it immediately while its syntax worker remains pending.
  if (node.content) buffer.setText(node.content)
}

export const createTranscriptMarkdownNodeRenderer = (renderer: ConstructorParameters<typeof CodeRenderable>[0]): NonNullable<MarkdownOptions["renderNode"]> => (token, context) => {
  if (token.type === "code") {
    return new TranscriptFenceRenderable(renderer, token.text, token.lang, context)
  }
  const node = context.defaultRender()
  if (node instanceof CodeRenderable) {
    linkifyTranscriptFallback(node)
    // OpenTUI styles escapes but does not conceal their introducer. Only
    // parser-recognized prose escapes change; code spans/fences stay literal.
    const highlight = node.onHighlight
    node.onHighlight = async (highlights, context) => (await highlight?.(highlights, context) ?? highlights).map(value => {
      const [start, end, scope, meta] = value
      const escaped = context.content.slice(start, end)
      return scope === "string.escape" && /^\\[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]$/.test(escaped)
        ? [start, end, scope, { ...meta, conceal: escaped.slice(1) }]
        : value
    })
    const original = node.onChunks
    node.onChunks = async (chunks, details) => linkifyTranscriptChunks(await original?.(chunks, details) ?? chunks)
  }
  return node
}

type FenceContext = Parameters<NonNullable<MarkdownOptions["renderNode"]>>[1]

// Markdown reuses the same code node while a fence's language streams in.
// Plaintext has no grammar; use Markdown's chunk path without concealment
// so it preserves the exact code bytes and invokes native linkification.
function createTranscriptCodeNode(renderer: ConstructorParameters<typeof CodeRenderable>[0], content: string, lang: string | undefined, context: FenceContext): TranscriptCodeRenderable {
  const node = new TranscriptCodeRenderable(renderer, {
    content, filetype: linkableCodeFiletype(lang),
    syntaxStyle: context.syntaxStyle, conceal: false, streaming: true,
    ...(context.treeSitterClient ? { treeSitterClient: context.treeSitterClient } : {}), width: "100%",
    onChunks: chunks => linkifyTranscriptChunks(chunks),
  })
  linkifyTranscriptFallback(node)
  return node
}

// MP-08 / MP-10 (PANE-09, R16): a fence block that shows ```diff/```patch as
// a Codex-style diff and everything else as highlighted, linkified code. It
// accepts the properties Markdown sets on code blocks while they stream and
// swaps its child when the language changes to or from diff.
export class TranscriptFenceRenderable extends BoxRenderable {
  private fenceContent: string
  private fenceLanguage: string | undefined
  private code: TranscriptCodeRenderable | null = null
  private diff: TranscriptDiffRenderable | null = null

  constructor(renderer: ConstructorParameters<typeof CodeRenderable>[0], content: string, lang: string | undefined, private readonly fenceContext: FenceContext) {
    super(renderer, { width: "100%", flexDirection: "column" })
    this.fenceContent = content
    this.fenceLanguage = lang
    this.sync()
  }

  get content(): string { return this.fenceContent }
  set content(value: string) {
    this.fenceContent = value
    this.sync()
  }

  get filetype(): string | undefined { return this.fenceLanguage }
  set filetype(value: string | undefined) {
    this.fenceLanguage = value
    this.sync()
  }

  set syntaxStyle(value: FenceContext["syntaxStyle"]) { if (this.code) this.code.syntaxStyle = value }
  set conceal(_value: boolean) {}
  set drawUnstyledText(value: boolean) { if (this.code) this.code.drawUnstyledText = value }
  set streaming(value: boolean) { if (this.code) this.code.streaming = value }

  private sync(): void {
    if (isDiffFiletype(this.fenceLanguage)) {
      if (this.code) {
        this.remove(this.code.id)
        this.code.destroyRecursively()
        this.code = null
      }
      if (!this.diff) {
        this.diff = new TranscriptDiffRenderable(this.ctx)
        this.add(this.diff)
      }
      this.diff.setFiles(parseUnifiedDiff(this.fenceContent))
      return
    }
    if (this.diff) {
      this.remove(this.diff.id)
      this.diff.destroyRecursively()
      this.diff = null
    }
    if (!this.code) {
      this.code = createTranscriptCodeNode(this.ctx, this.fenceContent, this.fenceLanguage, this.fenceContext)
      this.add(this.code)
      return
    }
    this.code.content = this.fenceContent
    this.code.filetype = this.fenceLanguage
  }
}

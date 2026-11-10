import MarkdownIt from "markdown-it"
import type { MarkdownIt as MarkdownItInstance, StateBlock, StateCore, StateInline, Token } from "markdown-it"

import {
  cachedHighlight,
  codeHighlightKey,
  highlightLanguage,
  requestHighlight,
} from "./code-highlight.js"
import { renderUnifiedDiffHtml } from "./diff-render.js"
import { renderMarkdownImage, scheduleMarkdownImageScan } from "./markdown-images.js"
import { ensureMarkdownInteractions } from "./markdown-interactions.js"
import { fillMathSlots, markdownMath, type MathEnv } from "./markdown-math.js"
import { sanitizeMarkdownHtml } from "./markdown-sanitize.js"

// MP-08 / MP-10 (PANE-09, ROADMAP R4): GFM content renderer for assistant and
// tool-output bodies in Room agent panes. Provider HTML is never interpreted
// (`html: false`); the result is additionally sanitized (markdown-sanitize.ts).

const shellLanguages = new Set(["bash", "sh", "shell", "shellscript", "zsh"])
const shellCommandPattern = /^\s*(?:[$#]\s*)?(?:cd|pnpm|npm|yarn|bun|git|gh|ls|cat|sed|rg|grep|find|curl|ssh|scp|docker|kubectl|make|node|python|uv|cargo|go|swift|xcodebuild)\b/
const fileReferencePattern = /^\[([^\]\n]+)\]\s*\((<[^>\n]+>|(?:\\.|[^()\\\n])+(?:\((?:\\.|[^()\\\n])*\)(?:\\.|[^()\\\n])*)*)\)/
const escapedCodeSpanPattern = /^`((?:\\`|[^`\n])+?)`/
const taskMarkerPattern = /^\[([ xX])\][ \t]+/
const externalHrefPattern = /^(?:https?:\/\/|mailto:)/i
// Codex's visualize skill ends a reply with this line for the app to draw the HTML page it names.
const visualizeLinePattern = /^visualize(\{.*\})[ \t]*$/
// Codex wraps its visualize directive in private-use marks (U+E200 opens, U+E202 separates the name from its arguments, U+E201
// closes), and a stream may show it only partly wrapped. Its app hides them; here they keep the line from being read. The marks
// that hug a visualize directive are removed, in prose only: marks in code or in another directive are the reply's own characters.
const wrappedVisualizePattern = /[\uE200-\uE2FF]*visualize[\uE200-\uE2FF]*(\{[^\uE200-\uE2FF\n]*\})[\uE200-\uE2FF]*/g
const maxCachedRenders = 300

const markdown = createMarkdown()
const renderCache = new Map<string, string>()

export function renderMarkdownHtml(text: string): string {
  const cached = renderCache.get(text)
  if (cached !== undefined) {
    ensureMarkdownInteractions()
    scheduleMarkdownImageScan()
    return cached
  }
  ensureMarkdownInteractions()
  const env: MathEnv = {}
  const html = `<div class="freeform-markdown">${fillMathSlots(sanitizeMarkdownHtml(markdown.render(unwrapVisualizeDirectives(text), env)), env)}</div>`
  // Pending work may be dropped while its pane is detached. Re-rendering
  // must request it again instead of memoizing an unresolved placeholder.
  if (!/data-(?:code-highlight|math-state)="pending"/.test(html)) renderCache.set(text, html)
  if (renderCache.size > maxCachedRenders) {
    const oldest = renderCache.keys().next().value
    if (oldest !== undefined) renderCache.delete(oldest)
  }
  return html
}

export function renderMarkdownInlineHtml(text: string): string {
  const env: MathEnv = {}
  return fillMathSlots(sanitizeMarkdownHtml(markdown.renderInline(unwrapVisualizeDirectives(text), env)), env)
}

function unwrapVisualizeDirectives(text: string): string {
  if (!/[\uE200-\uE2FF]/.test(text)) return text
  const code = new Set<number>()
  for (const token of markdown.parse(text, {})) {
    if ((token.type === "fence" || token.type === "code_block") && token.map) {
      for (let line = token.map[0]; line < token.map[1]; line += 1) code.add(line)
    }
  }
  // An inline code span may run over several lines of one paragraph, so each paragraph is transformed as a whole.
  const lines = text.split("\n")
  const output: string[] = []
  for (let index = 0; index < lines.length;) {
    if (code.has(index) || lines[index]!.trim() === "") { output.push(lines[index]!); index += 1; continue }
    let end = index
    while (end < lines.length && !code.has(end) && lines[end]!.trim() !== "") end += 1
    output.push(outsideCodeSpans(lines.slice(index, end).join("\n"), plain => plain.replace(wrappedVisualizePattern, "visualize$1")))
    index = end
  }
  return output.join("\n")
}

// Applies `transform` to the text between inline code spans. Openers and closers follow the renderer: a backtick the text escapes (\`)
// is not an opener, a span that quotes an escaped backtick ends at the first unescaped one, and any other span ends at the next
// backtick run of equal length.
function outsideCodeSpans(text: string, transform: (text: string) => string): string {
  let output = ""
  let plainStart = 0
  let index = 0
  while (index < text.length) {
    if (text[index] !== "`") { index += 1; continue }
    if (isEscaped(text, index)) { index += 1; continue }
    const quoting = escapedCodeSpanPattern.exec(text.slice(index))
    if (quoting?.[1]!.includes("\\`")) {
      output += transform(text.slice(plainStart, index)) + quoting[0]
      index += quoting[0].length
      plainStart = index
      continue
    }
    let runEnd = index
    while (text[runEnd] === "`") runEnd += 1
    const run = text.slice(index, runEnd)
    let close = runEnd
    for (;;) {
      close = text.indexOf(run, close)
      if (close < 0 || (text[close - 1] !== "`" && text[close + run.length] !== "`")) break
      close += 1
    }
    if (close < 0) { index = runEnd; continue }
    output += transform(text.slice(plainStart, index)) + text.slice(index, close + run.length)
    index = close + run.length
    plainStart = index
  }
  return output + transform(text.slice(plainStart))
}

function isEscaped(text: string, index: number): boolean {
  let slashes = 0
  while (text[index - 1 - slashes] === "\\") slashes += 1
  return slashes % 2 === 1
}

function createMarkdown(): MarkdownItInstance {
  const md = new MarkdownIt("default", {
    html: false,
    linkify: true,
    breaks: true,
    typographer: false,
  })
  md.linkify.set({ fuzzyLink: false, fuzzyEmail: false })
  md.use(markdownMath)
  md.block.ruler.before("paragraph", "visualize_page", visualizeLineRule, { alt: ["paragraph"] })
  md.inline.ruler.before("link", "file_reference", fileReferenceRule)
  md.inline.ruler.before("backticks", "escaped_code_span", escapedCodeSpanRule)
  md.core.ruler.after("inline", "fence_snippet", fenceSnippetRule)
  md.core.ruler.after("fence_snippet", "task_list", taskListRule)

  const rules = md.renderer.rules
  rules.fence = (tokens, index) => renderCodeFigure(tokens[index]!.content, tokens[index]!.info)
  rules.code_block = (tokens, index) => renderCodeFigure(tokens[index]!.content, "")
  rules.visualize_page = (tokens, index) => {
    const path = (tokens[index]!.meta as { path: string }).path
    return `<p class="freeform-md-visual" title="${escapeHtml(path)}">▣ visual: ${escapeHtml(path.split("/").pop() || path)} · ${escapeHtml(path)} (open in the web app)</p>`
  }
  rules.file_reference = (tokens, index) => {
    const { label, target } = tokens[index]!.meta as { label: string; target: string }
    return `<span class="freeform-code-reference" role="link" tabindex="0" data-file-reference-target="${escapeHtml(target)}" aria-label="File reference: ${escapeHtml(label)}">${escapeHtml(label)}</span>`
  }
  rules.task_checkbox = (tokens, index) => {
    const checked = tokens[index]!.meta?.checked === true
    return `<span class="freeform-md-task" role="checkbox" aria-checked="${checked}" aria-disabled="true" data-checked="${checked}"></span>`
  }
  rules.link_open = (tokens, index, options, env, self) => {
    const token = tokens[index]!
    const href = String(token.attrGet("href") ?? "")
    if (!externalHrefPattern.test(href)) {
      // Relative, anchor and other-scheme targets have no meaning inside a
      // pane; keep the label readable and never navigate the app.
      token.meta = { ...token.meta, inert: true }
      return `<span class="freeform-md-link-inert" title="${escapeHtml(href)}">`
    }
    token.attrSet("target", "_blank")
    token.attrSet("rel", "noopener noreferrer nofollow")
    token.attrJoin("class", "freeform-md-link")
    if (!token.attrGet("title")) token.attrSet("title", href)
    return self.renderToken(tokens, index, options)
  }
  rules.link_close = (tokens, index, options, env, self) => {
    const opener = findLinkOpen(tokens, index)
    return opener?.meta?.inert ? "</span>" : self.renderToken(tokens, index, options)
  }
  rules.image = (tokens, index) => {
    const token = tokens[index]!
    const alt = token.children?.map((child) => child.content).join("") || token.content
    return renderMarkdownImage(String(token.attrGet("src") ?? ""), alt, escapeHtml)
  }
  rules.table_open = () => `<div class="freeform-md-table"><div class="freeform-md-table-bar"><button type="button" class="freeform-md-copy" data-markdown-copy="table" aria-label="Copy table">copy</button></div><div class="freeform-md-table-scroll" tabindex="0"><table>`
  rules.table_close = () => "</table></div></div>"
  return md
}

function renderCodeFigure(content: string, info: string): string {
  const code = content.replace(/\r?\n$/, "")
  const declared = info.trim().split(/\s+/)[0] ?? ""
  const language = declared || inferFenceLanguage(code)
  const label = shellLanguages.has(language.toLowerCase()) ? "bash" : language
  if (/^(?:diff|patch)$/i.test(language)) {
    return `<figure class="freeform-terminal-code freeform-diff-figure" data-language="${escapeHtml(language)}"><figcaption><button type="button" class="freeform-md-copy" data-markdown-copy="code" aria-label="Copy diff">copy</button></figcaption>${renderUnifiedDiffHtml(code)}<pre hidden><code data-copy-source>${escapeHtml(code)}</code></pre></figure>`
  }
  const highlight = highlightLanguage(language)
  let codeAttributes = ""
  let inner = escapeHtml(code)
  if (highlight) {
    const key = codeHighlightKey(code, highlight)
    const cached = cachedHighlight(key)
    if (typeof cached === "string") {
      inner = cached
      codeAttributes = ` data-code-key="${key}" data-code-highlight="done"`
    } else if (cached === null) {
      codeAttributes = ` data-code-key="${key}" data-code-highlight="none"`
    } else {
      requestHighlight(key, code, highlight)
      codeAttributes = ` data-code-key="${key}" data-code-highlight="${cachedHighlight(key) === null ? "none" : "pending"}"`
    }
  }
  return `<figure class="freeform-terminal-code" data-language="${escapeHtml(language)}"><figcaption><span>${escapeHtml(label)}</span><button type="button" class="freeform-md-copy" data-markdown-copy="code" aria-label="Copy code">copy</button></figcaption><pre tabindex="0" aria-label="${escapeHtml(label)} code output"><code${codeAttributes}>${inner}</code></pre></figure>`
}

function inferFenceLanguage(code: string): string {
  return shellCommandPattern.test(code) ? "bash" : "text"
}

function findLinkOpen(tokens: Token[], closeIndex: number): Token | null {
  for (let index = closeIndex - 1; index >= 0; index -= 1) {
    if (tokens[index]!.type === "link_open") return tokens[index]!
  }
  return null
}

// This pane does not draw the page a visualize line names; it names the page,
// like the terminal does. The line ends a paragraph; code keeps it literal.
function visualizeLineRule(state: StateBlock, startLine: number, _endLine: number, silent: boolean): boolean {
  if (state.sCount[startLine]! - state.blkIndent >= 4) return false
  const line = state.src.slice(state.bMarks[startLine]! + state.tShift[startLine]!, state.eMarks[startLine])
  const json = visualizeLinePattern.exec(line)?.[1]
  const path = json === undefined ? null : visualizePagePath(json)
  if (path === null) return false
  if (silent) return true
  const token = state.push("visualize_page", "", 0)
  token.meta = { path }
  token.map = [startLine, startLine + 1]
  state.line = startLine + 1
  return true
}

function visualizePagePath(json: string): string | null {
  let path: unknown
  try { path = (JSON.parse(json) as { path?: unknown }).path } catch { return null }
  return typeof path === "string" && path.trim() ? path.trim() : null
}

// Local absolute paths keep the pane's file-reference behaviour, including
// targets with spaces/parentheses and a label wrapped onto the next line.
function fileReferenceRule(state: StateInline, silent: boolean): boolean {
  if (state.src.charCodeAt(state.pos) !== 0x5b /* [ */) return false
  const match = fileReferencePattern.exec(state.src.slice(state.pos, state.posMax))
  if (!match) return false
  const label = match[1] ?? ""
  const target = (match[2] ?? "").trim().replace(/^<|>$/g, "")
  if (!label.trim() || !target.startsWith("/") || target.startsWith("//")) return false
  if (!silent) {
    const token = state.push("file_reference", "", 0)
    token.meta = { label, target }
  }
  state.pos += match[0].length
  return true
}

// Agents write `a \`b\`` inside single-backtick spans; CommonMark would end the
// span at the escaped backtick. Only spans that contain `\`` take this path.
function escapedCodeSpanRule(state: StateInline, silent: boolean): boolean {
  if (state.src.charCodeAt(state.pos) !== 0x60 /* ` */) return false
  if (state.pos > 0 && state.src.charCodeAt(state.pos - 1) === 0x60) return false
  const match = escapedCodeSpanPattern.exec(state.src.slice(state.pos, state.posMax))
  if (!match || !match[1]!.includes("\\`")) return false
  if (!silent) {
    const token = state.push("code_inline", "code", 0)
    token.markup = "`"
    token.content = match[1]!.replace(/\\`/g, "`")
  }
  state.pos += match[0].length
  return true
}

// A paragraph that is only ```snippet``` is shown as a code frame, matching
// how agents use single-line triple backticks.
function fenceSnippetRule(state: StateCore): void {
  const tokens = state.tokens
  for (let index = 0; index + 2 < tokens.length; index += 1) {
    const [open, inline, close] = [tokens[index]!, tokens[index + 1]!, tokens[index + 2]!]
    if (open.type !== "paragraph_open" || inline.type !== "inline" || close.type !== "paragraph_close") continue
    const children = inline.children ?? []
    if (children.length !== 1 || children[0]!.type !== "code_inline" || children[0]!.markup.length < 3) continue
    const fence = new state.Token("fence", "code", 0)
    fence.content = children[0]!.content
    fence.info = ""
    fence.markup = "```"
    fence.block = true
    fence.map = open.map
    tokens.splice(index, 3, fence)
  }
}

function taskListRule(state: StateCore): void {
  const tokens = state.tokens
  for (let index = 2; index < tokens.length; index += 1) {
    const inline = tokens[index]!
    if (inline.type !== "inline" || tokens[index - 1]!.type !== "paragraph_open" || tokens[index - 2]!.type !== "list_item_open") continue
    const first = inline.children?.[0]
    const marker = first?.type === "text" ? taskMarkerPattern.exec(first.content) : null
    if (!first || !marker) continue
    first.content = first.content.slice(marker[0].length)
    const checkbox = new state.Token("task_checkbox", "", 0)
    checkbox.meta = { checked: marker[1] !== " " }
    inline.children!.unshift(checkbox)
    tokens[index - 2]!.attrJoin("class", "freeform-md-task-item")
    const list = findEnclosingListOpen(tokens, index - 2)
    list?.attrJoin("class", "freeform-md-task-list")
  }
}

function findEnclosingListOpen(tokens: Token[], itemIndex: number): Token | null {
  const level = tokens[itemIndex]!.level - 1
  for (let index = itemIndex - 1; index >= 0; index -= 1) {
    const token = tokens[index]!
    if ((token.type === "bullet_list_open" || token.type === "ordered_list_open") && token.level === level) {
      return String(token.attrGet("class") ?? "").includes("freeform-md-task-list") ? null : token
    }
  }
  return null
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (match) => {
    const escapes: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }
    return escapes[match] ?? match
  })
}

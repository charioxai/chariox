import type { MarkdownIt, StateBlock, StateInline, Token } from "markdown-it"

import { cachedMath, mathKey, requestMath } from "./math-render.js"

// MP-08 / MP-10 (PANE-09): strict math delimiters for agent Markdown.
// Display: `$$…$$` and `\[…\]` (own block or inline). Inline: `\(…\)`, and
// `$…$` only when unambiguous (pandoc rules): no space after the opening or
// before the closing `$`, no digit right after the closing `$`, no word
// character right before the opening `$`, one line. Code spans and fences are
// tokenized by markdown-it before these rules ever see their content.

export type MathEnv = { mathSlots?: string[] }

const inlineDollarPattern = /^\$((?:\\\$|[^$\n])+?)\$/

export function markdownMath(md: MarkdownIt): void {
  md.inline.ruler.before("escape", "math_inline", mathInlineRule)
  md.block.ruler.before("fence", "math_block", mathBlockRule, { alt: ["paragraph", "reference", "blockquote", "list"] })
  md.renderer.rules.math_inline = (tokens, index, _options, env) => renderMathToken(tokens[index]!, env as MathEnv, false)
  md.renderer.rules.math_block = (tokens, index, _options, env) => renderMathToken(tokens[index]!, env as MathEnv, true)
}

// After sanitizing, swap slot placeholders for KaTeX HTML (trust: false, its
// own escaping), which the generic allowlist would otherwise flatten.
export function fillMathSlots(html: string, env: MathEnv): string {
  const slots = env.mathSlots ?? []
  return slots.length === 0 ? html : html.replace(/<span data-math-slot="(\d+)"><\/span>/g, (match, index: string) => slots[Number(index)] ?? match)
}

function renderMathToken(token: Token, env: MathEnv, block: boolean): string {
  const display = token.meta?.display === true
  const tex = token.content
  const key = mathKey(tex, display)
  const cached = cachedMath(key)
  const tag = block ? "div" : "span"
  const classes = display ? "freeform-md-math freeform-md-math-display" : "freeform-md-math"
  if (typeof cached === "string") {
    env.mathSlots ??= []
    env.mathSlots.push(cached)
    return `<${tag} class="${classes}" data-math-key="${key}" data-math-state="done"><span data-math-slot="${env.mathSlots.length - 1}"></span></${tag}>`
  }
  if (cached === undefined) requestMath(key, tex, display)
  const state = cached === null || cachedMath(key) === null ? "error" : "pending"
  return `<${tag} class="${classes}" data-math-key="${key}" data-math-state="${state}">${escapeHtml(token.markup)}${escapeHtml(tex)}${escapeHtml(closingMarkup(token.markup))}</${tag}>`
}

function mathInlineRule(state: StateInline, silent: boolean): boolean {
  const src = state.src
  const pos = state.pos
  const max = state.posMax
  const rest = src.slice(pos, max)
  let open = ""
  let close = ""
  if (rest.startsWith("\\(")) [open, close] = ["\\(", "\\)"]
  else if (rest.startsWith("\\[")) [open, close] = ["\\[", "\\]"]
  else if (rest.startsWith("$$")) [open, close] = ["$$", "$$"]
  else if (rest.startsWith("$")) return dollarInline(state, silent)
  else return false
  const end = src.indexOf(close, pos + open.length)
  if (end < 0 || end + close.length > max) return false
  const tex = src.slice(pos + open.length, end)
  if (!tex.trim()) return false
  if (!silent) pushMath(state, tex, open, open !== "\\(")
  state.pos = end + close.length
  return true
}

function dollarInline(state: StateInline, silent: boolean): boolean {
  const src = state.src
  const pos = state.pos
  const before = pos > 0 ? src[pos - 1]! : ""
  if (/[\p{L}\p{N}$\\]/u.test(before)) return false
  const match = inlineDollarPattern.exec(src.slice(pos, state.posMax))
  if (!match) return false
  const tex = match[1]!
  const after = src[pos + match[0].length] ?? ""
  if (/^\s/.test(tex) || /\s$/.test(tex) || tex.endsWith("\\") || /\d/.test(after)) return false
  if (!silent) pushMath(state, tex, "$", false)
  state.pos += match[0].length
  return true
}

function pushMath(state: StateInline, tex: string, markup: string, display: boolean): void {
  const token = state.push("math_inline", "math", 0)
  token.content = tex
  token.markup = markup
  token.meta = { display }
}

function mathBlockRule(state: StateBlock, startLine: number, endLine: number, silent: boolean): boolean {
  const start = state.bMarks[startLine]! + state.tShift[startLine]!
  if (state.sCount[startLine]! - state.blkIndent >= 4) return false
  const firstLine = state.src.slice(start, state.eMarks[startLine])
  const open = firstLine.startsWith("$$") ? "$$" : firstLine.startsWith("\\[") ? "\\[" : null
  if (!open) return false
  const close = closingMarkup(open)
  let content = firstLine.slice(open.length)
  let line = startLine
  let closed = false
  for (;;) {
    const end = content.lastIndexOf(close)
    if (end >= 0 && content.slice(end + close.length).trim() === "") {
      content = content.slice(0, end)
      closed = true
      break
    }
    line += 1
    if (line >= endLine) break
    const next = state.src.slice(state.bMarks[line]! + state.tShift[line]!, state.eMarks[line])
    if (state.isEmpty(line)) break
    content += `\n${next}`
  }
  if (!closed || !content.trim()) return false
  if (silent) return true
  const token = state.push("math_block", "math", 0)
  token.block = true
  token.content = content.trim()
  token.markup = open
  token.meta = { display: true }
  token.map = [startLine, line + 1]
  state.line = line + 1
  return true
}

function closingMarkup(open: string): string {
  return open === "\\(" ? "\\)" : open === "\\[" ? "\\]" : open
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (match) => `&#${match.charCodeAt(0)};`)
}

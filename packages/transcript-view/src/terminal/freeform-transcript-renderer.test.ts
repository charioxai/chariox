import assert from "node:assert/strict"
import test from "node:test"
import { renderMarkdownHtml, renderMarkdownInlineHtml } from "../markdown/markdown-renderer.js"
import { renderFreeformMessage, renderFreeformTranscriptEntry } from "./freeform-transcript-renderer.js"
import { renderPatchFilesDiffHtml } from "../markdown/diff-render.js"
import type { TerminalOutputEntry } from "./runtime-state.js"

// MP-08: a pane that cannot draw the page a visualize line names shows one calm line instead of raw JSON.
test("a reply ending in a visualize line names the page, while a fenced one stays code", () => {
  for (const text of [
    'Here is the chart.\n\nvisualize{"path":"/w/chart.html"}',
    'Here is the chart.\nvisualize{"path":"/w/chart.html","mode":"wide"}',
  ]) {
    const html = renderFreeformMessage("agent", text)
    assert.match(html, /<p>Here is the chart\.<\/p>/)
    assert.match(html, /▣ visual: chart\.html · \/w\/chart\.html \(open in the web app\)/)
    assert.doesNotMatch(html, /visualize\{|<img/)
  }
  const fenced = renderFreeformMessage("agent", '```\nvisualize{"path":"/w/chart.html"}\n```')
  assert.match(fenced, /<code[^>]*>visualize\{&quot;path&quot;:&quot;\/w\/chart\.html&quot;\}<\/code>/)
  assert.doesNotMatch(fenced, /▣ visual/)
  assert.match(renderFreeformMessage("agent", 'visualize{"path": 3}'), /<p>visualize\{&quot;path&quot;: 3\}<\/p>/, "not a page: left as written")
})

// Codex's own visualize skill wraps its directive in private-use marks (U+E200 opens, U+E202 separates the name from its
// arguments, U+E201 closes). They are invisible in most fonts, so the pane showed a raw line and named no page.
test("a visualize line in Codex's wrapper marks names the page, and no private-use mark is ever shown", () => {
  const wrapped = 'visualize{"path":"/Users/miguel/chariox-workspace/output/aa-recreation.html"}'
  for (const text of [
    `Here is the recreation.\n\n${wrapped}`,
    `Here is the recreation.\n${wrapped}  `,
    'visualize{"path":"/w/chart.html"}',
    'visualize{"path":"/w/chart.html"}',
    'visualize{"path":"/w/chart.html"}',
    `Done:\n\n> ${wrapped}`,
  ]) {
    const html = renderFreeformMessage("agent", text)
    assert.match(html, /▣ visual: [\w.-]+\.html · \/[\w/.-]+\.html \(open in the web app\)/, JSON.stringify(text))
    assert.doesNotMatch(html, /visualize|[-]/, JSON.stringify(text))
  }
  const other = renderFreeformMessage("agent", `A note stays.\n\nvisualize{"mode":"wide"}`)
  assert.doesNotMatch(other, /[-]/)
  assert.match(other, /A note stays\./)
})

// Review of #1326: a tool row that has nothing to open to (a web search that is still running) is a title row with no toggle.
test("a tool row with no content has no disclosure toggle, one with content keeps it", () => {
  const entry = (record: object): TerminalOutputEntry => ({ key: "t1", role: "tool", agentId: "a", text: JSON.stringify(record) })
  const running = renderFreeformTranscriptEntry(entry({ id: "w1", tool: "web_search", status: "running" }))
  assert.match(running, /freeform-blob-title/, "still a title row")
  assert.doesNotMatch(running, /data-freeform-blob-toggle|freeform-blob-chevron|aria-expanded/)
  const done = renderFreeformTranscriptEntry(entry({ id: "w1", tool: "web_search", status: "completed", input: { query: "alhambra" } }))
  assert.match(done, /data-freeform-blob-toggle/)
  assert.match(done, /freeform-blob-chevron/)
})

// Review of #1326: a binary write has no diff, only a note (its size); the web diff view shows the note.
test("a binary write shows its note in the diff view, a move keeps its title", () => {
  const html = renderPatchFilesDiffHtml([{ kind: "update", filePath: "out/logo.png", title: "Wrote out/logo.png", diff: null, note: "Binary file · 3 KB" }])
  assert.match(html, /Binary file · 3 KB/)
  assert.doesNotMatch(html, /no textual changes/)
  const moved = renderPatchFilesDiffHtml([{ kind: "move", filePath: "a.txt", title: "Moved a.txt -> b.txt", diff: null, note: "Moved to b.txt" }])
  assert.match(moved, /Moved a\.txt -&#62; b\.txt/)
  assert.equal(moved.match(/Moved to b\.txt/g)?.length ?? 0, 0, "the title already says it")
})

// Only the visualize wrapper is a directive the pane reads. Text in code, and marks of any other directive, are the reply's own
// characters: removing them from the whole source would change what a code sample or a quotation says.
test("private-use marks in code spans, fences and other directives are left as written", () => {
  const mark = (name: string, args: string) => `\uE200${name}\uE202${args}\uE201`
  const wrapped = mark("visualize", '{"path":"/w/chart.html"}')
  const span = renderFreeformMessage("agent", `Codex writes \`${wrapped}\` for it.`)
  assert.match(span, /<code>\uE200visualize\uE202\{[^<]*\uE201<\/code>/, "the code span is untouched")
  assert.doesNotMatch(span, /▣ visual/)
  const fence = renderFreeformMessage("agent", `\`\`\`text\n${wrapped}\n\`\`\``)
  assert.match(fence, /\uE200visualize\uE202[^<]*\uE201/, "the fence is untouched")
  assert.doesNotMatch(fence, /▣ visual/)
  const other = mark("cite", "turn0search0")
  assert.ok(renderFreeformMessage("agent", `Sourced ${other}.`).includes(other), "another directive is not rewritten")
})

// c#612/#1323 review: an inline code span may run over several lines of a paragraph. Reading the paragraph line by line took its first
// line for plain text and removed the marks inside the span, which changed what the code says.
test("private-use marks inside an inline code span that runs over several lines are left as written", () => {
  const mark = (name: string, args: string) => `\uE200${name}\uE202${args}\uE201`
  const wrapped = mark("visualize", '{"path":"/w/chart.html"}')
  const text = `Codex writes \`the call\n${wrapped}\nends here\` for it, and ${wrapped} stays a directive.`
  for (const [name, html] of [["renderMarkdownHtml", renderMarkdownHtml(text)], ["renderMarkdownInlineHtml", renderMarkdownInlineHtml(text)]] as const) {
    const span = /<code>([^<]*)<\/code>/.exec(html)
    assert.ok(span, `${name}: the code span keeps its wrapper`)
    assert.ok(/\uE200visualize\uE202[^]*\uE201/.test(span[1]!), `${name}: the marks inside the span are untouched`)
    const rest = html.slice(span.index + span[0].length)
    assert.ok(!/[\uE200-\uE2FF]/.test(rest), `${name}: the marks around a directive outside the span are removed`)
  }
})

// #1323 review: a backtick the reply escapes (\`) is text, not the opener of a code span. Taking it for one paired it with the opener of a
// real span on a later line of the paragraph and removed the marks inside that span.
test("an escaped backtick on an earlier line does not pair with a real code span's opener", () => {
  const mark = (name: string, args: string) => `\uE200${name}\uE202${args}\uE201`
  const wrapped = mark("visualize", '{"path":"/w/chart.html"}')
  const text = `Write \\\` to quote text:\nCodex writes \`the call ${wrapped} ends\` for it, and ${wrapped} stays a directive.`
  for (const [name, html] of [["renderMarkdownHtml", renderMarkdownHtml(text)], ["renderMarkdownInlineHtml", renderMarkdownInlineHtml(text)]] as const) {
    const span = /<code>([^<]*)<\/code>/.exec(html)
    assert.ok(span, `${name}: the code span keeps its wrapper`)
    assert.ok(/\uE200visualize\uE202[^]*\uE201/.test(span[1]!), `${name}: the marks inside the span are untouched`)
    const rest = html.slice(span.index + span[0].length)
    assert.ok(!/[\uE200-\uE2FF]/.test(rest), `${name}: the marks around a directive outside the span are removed`)
  }
})

test("a code span that quotes an escaped backtick keeps the marks inside it", () => {
  const wrapped = `\uE200visualize\uE202{"path":"/w/chart.html"}\uE201`
  const html = renderMarkdownHtml(`Use \`a \\\` ${wrapped}\` here, then ${wrapped}.`)
  const span = /<code>([^<]*)<\/code>/.exec(html)
  assert.ok(span && /\uE200visualize/.test(span[1]!), "the marks inside the span are untouched")
  assert.ok(!/[\uE200-\uE2FF]/.test(html.slice(span.index + span[0].length)), "the directive after the span is unwrapped")
})

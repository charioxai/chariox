import assert from "node:assert/strict"
import test from "node:test"
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
  const other = renderFreeformMessage("agent", `A note stays.\n\nvisualize{"mode":"wide"}`)
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

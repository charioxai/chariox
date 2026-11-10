import assert from "node:assert/strict"
import test from "node:test"
import { renderFreeformMessage } from "./freeform-transcript-renderer.js"

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

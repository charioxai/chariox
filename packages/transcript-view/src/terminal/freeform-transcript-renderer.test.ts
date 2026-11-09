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

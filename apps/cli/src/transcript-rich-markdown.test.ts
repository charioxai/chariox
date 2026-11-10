import assert from "node:assert/strict"
import test from "node:test"

import { mapMarkdownProse, mapOutsideCodeSpans, prepareTranscriptMarkdown } from "./transcript-rich-markdown.js"

// MP-08 / MP-10 (PANE-09): terminal rendering of rich Markdown.

test("task list markers render as checkboxes in bullet and ordered lists", () => {
  assert.equal(
    prepareTranscriptMarkdown("- [ ] todo\n- [x] done\n  * [X] nested\n1. [ ] first\n- plain [ ] text"),
    "- ☐ todo\n- ☑ done\n  * ☑ nested\n1. ☐ first\n- plain [ ] text",
  )
})

test("task markers inside fenced or inline code stay literal", () => {
  const text = "```md\n- [ ] in code\n```\n- [ ] real\n~~~\n- [x] tilde code\n~~~\n`- [ ] inline`"
  assert.equal(prepareTranscriptMarkdown(text), "```markdown\n- [ ] in code\n```\n- ☐ real\n~~~\n- [x] tilde code\n~~~\n`- [ ] inline`")
})

test("an unterminated streaming fence protects everything after it", () => {
  assert.equal(prepareTranscriptMarkdown("- [ ] a\n```\n- [ ] b"), "- ☐ a\n```\n- [ ] b")
})

test("prose mapping treats one-line triple-backtick snippets as prose", () => {
  const seen: string[] = []
  mapMarkdownProse("```npm test```\nafter\n````\n```\nstill code\n````\nend", (prose) => {
    seen.push(prose)
    return prose
  })
  assert.deepEqual(seen, ["```npm test```\nafter", "end"])
})

test("code spans of equal backtick runs are protected; unmatched backticks are text", () => {
  const upper = (text: string) => text.toUpperCase()
  assert.equal(mapOutsideCodeSpans("a `b` c ``d ` e`` f ` g", upper), "A `b` C ``d ` e`` F ` G")
})

test("images become a one-line placeholder with name, host and an open link", () => {
  assert.equal(
    prepareTranscriptMarkdown("![Wikimedia logo](https://upload.wikimedia.org/wikipedia/commons/8/81/Wikimedia-logo.svg \"logo\")"),
    "[▣ image: Wikimedia logo · Wikimedia-logo.svg · upload.wikimedia.org · size unknown (open link)](https://upload.wikimedia.org/wikipedia/commons/8/81/Wikimedia-logo.svg)",
  )
  assert.equal(
    prepareTranscriptMarkdown("![dot](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==)"),
    "▣ image: dot · png 70 B (inline; open in the web app)",
  )
  assert.equal(prepareTranscriptMarkdown("![plot v2](./out/plot.png)"), "▣ image: plot v2 · ./out/plot.png · size unknown (open in the web app)")
  assert.equal(prepareTranscriptMarkdown("`![code](x.png)`\n```\n![fence](y.png)\n```"), "`![code](x.png)`\n```\n![fence](y.png)\n```")
})

// MP-08 / MP-10 (PANE-09): Markdown code containers must remain literal.
test("quoted and indented code protect math, images and task syntax", () => {
  const quoted = "> ```text\n> $x^2$ ![image](x.png)\n> - [x] literal\n> ```"
  const indented = "    $x^2$ ![image](x.png)\n    - [x] literal"
  const nested = "- example:\n\n    ```text\n    $x^2$ ![image](x.png)\n    - [x] literal\n    ```"
  for (const code of [quoted, indented, nested]) assert.equal(prepareTranscriptMarkdown(code), code)
  assert.equal(prepareTranscriptMarkdown("> $x^2$\n\n- [x] real"), "> x²\n\n- ☑ real")
})

test("MP-08 a visualize line names the page the web pane draws, and stays literal in code", () => {
  assert.equal(
    prepareTranscriptMarkdown('Here is the chart.\n\nvisualize{"path":"/Users/miguel/arroba-cloud/tmp/aa-index-reproduction.html"}'),
    "Here is the chart.\n\n▣ visual: aa-index-reproduction.html · /Users/miguel/arroba-cloud/tmp/aa-index-reproduction.html (open in the web app)",
  )
  assert.equal(prepareTranscriptMarkdown('```\nvisualize{"path":"a.html"}\n```'), '```\nvisualize{"path":"a.html"}\n```')
  assert.equal(prepareTranscriptMarkdown('visualize{"path": 3}'), 'visualize{"path": 3}', "not a page: left as written")
})

test("MP-08 a visualize line in Codex's wrapper marks names the page, and no private-use mark reaches the terminal", () => {
  const wrapped = 'visualize{"path":"/w/chart.html"}'
  assert.equal(
    prepareTranscriptMarkdown(`Here is the chart.\n\n${wrapped}`),
    "Here is the chart.\n\n▣ visual: chart.html · /w/chart.html (open in the web app)",
  )
  assert.equal(prepareTranscriptMarkdown("A note stays."), "A note stays.")
})

test("MP-08 marks in code spans, fences and other directives are left as written", () => {
  const mark = (name: string, args: string) => `\uE200${name}\uE202${args}\uE201`
  const wrapped = mark("visualize", '{"path":"/w/chart.html"}')
  assert.equal(prepareTranscriptMarkdown(`Codex writes \`${wrapped}\` for it.`), `Codex writes \`${wrapped}\` for it.`)
  assert.equal(prepareTranscriptMarkdown(`\`\`\`text\n${wrapped}\n\`\`\``), `\`\`\`text\n${wrapped}\n\`\`\``)
  assert.equal(prepareTranscriptMarkdown(`Sourced ${mark("cite", "turn0search0")}.`), `Sourced ${mark("cite", "turn0search0")}.`)
})

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

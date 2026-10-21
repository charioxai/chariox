import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, MarkdownRenderable, SyntaxStyle } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createTranscriptMarkdownNodeRenderer } from "./transcript-hyperlinks.js"
import { readApplyPatchFiles } from "@chariox/tool-display"
import { buildApplyPatchDiffContent } from "./transcript-diff-render.js"

// MP-08 / MP-10 (PANE-09, R16): Codex-style diffs in the terminal.
const frame = async (harness: Awaited<ReturnType<typeof createTestRenderer>>) => {
  for (let i = 0; i < 5; i++) await harness.renderOnce()
  return harness.captureCharFrame().split("\n").map(line => line.trimEnd()).filter(Boolean).join("\n")
}

test("a diff fence whose language streams in becomes a diff view", async () => {
  const harness = await createTestRenderer({ width: 60, height: 12, useThread: false })
  const syntax = SyntaxStyle.fromStyles({})
  const node = new MarkdownRenderable(harness.renderer, {
    content: "```d\n-old line\n+new", syntaxStyle: syntax, streaming: true,
    renderNode: createTranscriptMarkdownNodeRenderer(harness.renderer),
  })
  harness.renderer.root.add(node)
  try {
    assert.match(await frame(harness), /-old line/)
    node.content = "```diff\n@@ -4,2 +4,2 @@\n-old line\n+new line\n context\n```"
    assert.equal(await frame(harness), "• Diff (+1 −1)\n4 - old line\n4 + new line\n5   context")
    node.content = "```python\nprint(1)\n```"
    assert.match(await frame(harness), /print\(1\)/)
  } finally { harness.renderer.destroy(); syntax.destroy() }
})

test("tool patches show verb, per-file rows, gutter and hunk gaps", async () => {
  const harness = await createTestRenderer({ width: 60, height: 14, useThread: false })
  const body = new BoxRenderable(harness.renderer, { width: "100%" })
  harness.renderer.root.add(body)
  try {
    buildApplyPatchDiffContent(harness.renderer, body, [
      { kind: "update", filePath: "README.md", title: "Patched README.md", diff: "--- a/README.md\n+++ b/README.md\n@@ -1,2 +1,2 @@\n-before: original\n+before: updated\n keep\n@@ -9 +9 @@\n-x\n+y" },
      { kind: "add", filePath: "new.txt", title: "Created new.txt", diff: "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello" },
    ])
    assert.equal(await frame(harness), [
      "• Edited 2 files (+3 −2)",
      "  └ README.md (+2 −2)",
      "1 - before: original",
      "1 + before: updated",
      "2   keep",
      "  ⋮",
      "9 - x",
      "9 + y",
      "  └ new.txt (+1 −0)",
      "1 + hello",
    ].join("\n"))
  } finally { harness.renderer.destroy() }
})

// MP-08 / MP-10: exercise the actual range-free apply_patch producer.
test("apply_patch text retains hunk gaps without invented gutter numbers", async () => {
  const harness = await createTestRenderer({ width: 60, height: 12, useThread: false })
  const body = new BoxRenderable(harness.renderer, { width: "100%" })
  harness.renderer.root.add(body)
  try {
    const files = readApplyPatchFiles({ id: "patch", tool: "apply_patch", status: "completed", input: {
      patchText: "*** Begin Patch\n*** Update File: a.py\n@@\n keep\n-old\n+new\n@@ later\n other\n-x\n+y\n*** End Patch",
    } })
    buildApplyPatchDiffContent(harness.renderer, body, files)
    assert.equal(await frame(harness), ["• Edited a.py (+2 −2)", "   keep", " - old", " + new", " ⋮", "   other", " - x", " + y"].join("\n"))
  } finally { harness.renderer.destroy() }
})

// MP-08 / MP-10: mirror Cloud348 review's missing truncation notice.
test("a truncated tool diff keeps its warning in the TUI", async () => {
  const harness = await createTestRenderer({ width: 70, height: 10, useThread: false })
  const body = new BoxRenderable(harness.renderer, { width: "100%" })
  harness.renderer.root.add(body)
  try {
    buildApplyPatchDiffContent(harness.renderer, body, [{ kind: "update", filePath: "a.py", title: "Patched a.py (diff truncated)", diff: "--- a/a.py\n+++ b/a.py\n@@\n-old\n+new" }])
    assert.match(await frame(harness), /Patched a.py \(diff truncated\)/)
  } finally { harness.renderer.destroy() }
})

import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

import {
  buildApplyPatchNewPreview,
  formatToolDisplay,
  formatToolTranscriptUpdate,
  readApplyPatchFiles,
  type ToolTranscriptUpdate,
} from "./index.js"

async function fixture(path: string) {
  return JSON.parse(await readFile(new URL(`../fixtures/${path}`, import.meta.url), "utf8")) as {
    input: ToolTranscriptUpdate
    expect: Record<string, unknown>
  }
}

test("formatToolDisplay produces shared patch display with new-change preview", async () => {
  const { input, expect } = await fixture("apply_patch/update.json")
  const display = formatToolDisplay(input)
  const patch = display.blocks.find((block) => block.kind === "patch")

  assert.equal(display.version, 1)
  assert.equal(display.tool, "apply_patch")
  assert.equal(display.summary, expect.summary)
  assert.equal(patch?.kind, "patch")
  assert.deepEqual(patch?.files[0]?.previewLines, expect.previewLines)
  assert.equal(
    patch?.files[0]?.previewLines.some((line) => line.kind === "removed" || line.text.includes("oldValue")),
    false,
  )
})

test("formatToolDisplay summarizes shell commands without requiring client JSON parsing", async () => {
  const { input, expect } = await fixture("bash/command.json")
  const display = formatToolDisplay(input)

  assert.equal(display.summary, expect.summary)
  assert.deepEqual(display.blocks.map((block) => block.kind), expect.blocks)
})

test("formatToolDisplay normalizes OpenCode Arroba read tool aliases", async () => {
  const { input, expect } = await fixture("opencode/managed-read.json")
  const display = formatToolDisplay(input)

  assert.equal(display.summary, expect.summary)
  assert.deepEqual(display.blocks.map((block) => block.kind), expect.blocks)
  assert.match(JSON.stringify(display.blocks), /TOOL_DISPLAY_FIXTURE_SEED/)
})

test("formatToolDisplay normalizes OpenCode Arroba patch aliases", async () => {
  const { input, expect } = await fixture("opencode/managed-apply-patch.json")
  const display = formatToolDisplay(input)
  const patch = display.blocks.find((block) => block.kind === "patch")

  assert.equal(display.summary, expect.summary)
  assert.equal(patch?.kind, "patch")
  assert.deepEqual(patch?.files[0]?.previewLines, expect.previewLines)
})

test("formatToolDisplay accepts Codex runtime patch_text input", async () => {
  const { input, expect } = await fixture("codex/managed-apply-patch.json")
  const display = formatToolDisplay(input)
  const patch = display.blocks.find((block) => block.kind === "patch")

  assert.equal(display.summary, expect.summary)
  assert.equal(patch?.kind, "patch")
  assert.deepEqual(patch?.files[0]?.previewLines, expect.previewLines)
})

test("formatToolTranscriptUpdate keeps legacy markdown summaries stable", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-12",
      tool: "apply_patch",
      status: "completed",
      input: {
        patchText: [
          "*** Begin Patch",
          "*** Update File: src/app.ts",
          "@@",
          "-const oldValue = 1",
          "+const newValue = 2",
          "*** Delete File: src/old.ts",
          "*** End Patch",
        ].join("\n"),
      },
    }),
    [
      "**patch** · COMPLETED",
      "2 files · 1 updated, 1 deleted",
      "- Patched src/app.ts",
      "- Deleted src/old.ts",
    ].join("\n"),
  )
})

test("buildApplyPatchNewPreview hides old-side lines", () => {
  const [file] = readApplyPatchFiles({
    id: "tool-1",
    tool: "apply_patch",
    input: {
      patchText: [
        "*** Begin Patch",
        "*** Update File: src/app.ts",
        "@@",
        "-before()",
        "+after()",
        " context()",
        "*** End Patch",
      ].join("\n"),
    },
  })
  assert.ok(file)
  assert.deepEqual(buildApplyPatchNewPreview(file), [
    { kind: "meta", text: "@@ -1,2 +1,2 @@" },
    { kind: "added", text: "after()" },
    { kind: "context", text: "context()" },
  ])
})

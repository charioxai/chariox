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
  assert.equal(display.title, "read")
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

test("formatToolDisplay unwraps live MCP envelopes for Arroba read output", () => {
  const display = formatToolDisplay({
    id: "call_read",
    tool: "arroba.read_artifact",
    status: "completed",
    input: { path: "src/app.ts", domain: "text" },
    output: JSON.stringify({
      _meta: null,
      content: [
        {
          type: "text",
          text: JSON.stringify({
            content_text: "export const value = 1\n",
            domain: "text",
            path: "src/app.ts",
          }),
        },
      ],
    }),
  })

  assert.equal(display.title, "read")
  assert.equal(display.summary, "src/app.ts")
  assert.deepEqual(display.blocks, [
    { kind: "code", language: "typescript", text: "export const value = 1" },
  ])
})

test("formatToolDisplay unwraps live MCP envelopes for Arroba mutation output", () => {
  const display = formatToolDisplay({
    id: "call_write",
    tool: "arroba.write_artifact",
    status: "completed",
    input: { path: "src/app.ts", content_text: "next()\n", domain: "text" },
    output: JSON.stringify({
      _meta: null,
      content: [
        {
          type: "text",
          text: JSON.stringify({
            applied: true,
            change: {
              path: "src/app.ts",
              kind: "update",
              diff: "diff --git a/src/app.ts b/src/app.ts\n--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1,1 +1,1 @@\n-old()\n+next()",
              diff_truncated: false,
            },
          }),
        },
      ],
    }),
  })
  const patch = display.blocks.find((block) => block.kind === "patch")

  assert.equal(display.title, "patch")
  assert.equal(display.summary, "Patched src/app.ts")
  assert.equal(patch?.kind, "patch")
  assert.deepEqual(patch?.files[0]?.previewLines, [
    { kind: "meta", text: "@@ -1,1 +1,1 @@" },
    { kind: "added", text: "next()" },
  ])
})

test("formatToolDisplay distinguishes home-proxy tool calls", () => {
  const update: ToolTranscriptUpdate = {
    id: "tool-home-1",
    tool: "home_lookup",
    placement: "home-proxy",
    authority: "home",
    execution_location: "home",
    status: "completed",
    input: { query: "status" },
    output: "ok",
  }

  const markdown = formatToolTranscriptUpdate(update)
  const display = formatToolDisplay(update)

  assert.match(markdown, /^\*\*home_lookup\*\* · HOME-PROXY · COMPLETED/)
  assert.equal(display.title, "home-proxy · home_lookup")
  assert.equal(display.collapsed.title, "home-proxy · home_lookup · COMPLETED")
})

test("formatToolDisplay distinguishes worker-local tool calls", () => {
  const update: ToolTranscriptUpdate = {
    id: "tool-worker-1",
    tool: "worker_lookup",
    placement: "worker-local",
    authority: "worker",
    execution_location: "worker",
    status: "completed",
    input: { query: "status" },
    output: "ok",
  }

  const markdown = formatToolTranscriptUpdate(update)
  const display = formatToolDisplay(update)

  assert.match(markdown, /^\*\*worker_lookup\*\* · WORKER-LOCAL · COMPLETED/)
  assert.equal(display.title, "worker-local · worker_lookup")
  assert.equal(display.collapsed.title, "worker-local · worker_lookup · COMPLETED")
})

test("formatToolDisplay distinguishes skill snapshot tool calls", () => {
  const update: ToolTranscriptUpdate = {
    id: "tool-skill-1",
    tool: "skill_context",
    placement: "skill snapshot",
    authority: "home",
    execution_location: "none",
    status: "completed",
    text: "loaded",
  }

  const markdown = formatToolTranscriptUpdate(update)
  const display = formatToolDisplay(update)

  assert.match(markdown, /^\*\*skill_context\*\* · SKILL SNAPSHOT · COMPLETED/)
  assert.equal(display.title, "skill snapshot · skill_context")
  assert.equal(display.collapsed.title, "skill snapshot · skill_context · COMPLETED")
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

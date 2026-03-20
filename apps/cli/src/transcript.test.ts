import test from "node:test"
import assert from "node:assert/strict"

import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  shouldRenderProviderStatus,
  splitInlineCodeSpans,
} from "./transcript.js"

test("parseToolTranscriptUpdate reads structured tool payloads", () => {
  const parsed = parseToolTranscriptUpdate('{"id":"tool-1","tool":"bash","status":"running"}')
  assert.deepEqual(parsed, {
    id: "tool-1",
    tool: "bash",
    status: "running",
  })
})

test("formatToolTranscriptUpdate renders bash command inline with output", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-1",
      tool: "bash",
      status: "completed",
      input: { command: "git status" },
      output: "On branch main",
      description: "Shows working tree status",
    }),
    [
      "bash",
      "Shows working tree status",
      "$ git status",
      "On branch main",
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate falls back to rendered input and errors", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-2",
      tool: "read",
      status: "error",
      input: { filePath: "/tmp/demo.txt" },
      error: "file not found",
    }),
    [
      "read [error]",
      '{\n  "filePath": "/tmp/demo.txt"\n}',
      "Error: file not found",
    ].join("\n\n"),
  )
})

test("formatToolTranscriptUpdate renders todos as a checklist", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-3",
      tool: "todowrite",
      status: "completed",
      input: {
        todos: [
          {
            content: "Remove temporary idle-status debug logs from CLI and daemon",
            priority: "high",
            status: "completed",
          },
          {
            content: "Run CLI and daemon tests after log cleanup",
            priority: "medium",
            status: "pending",
          },
        ],
      },
    }),
    [
      "Todos: 1 todo remaining",
      "[✓] Remove temporary idle-status debug logs from CLI and daemon",
      "[ ] Run CLI and daemon tests after log cleanup",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders read output with a compact header", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-4",
      tool: "read",
      status: "completed",
      input: {
        filePath: "apps/daemon/src/provider/service.rs",
        offset: 480,
        limit: 220,
      },
      output: [
        "<path>/Users/miguel/arroba/apps/daemon/src/provider/service.rs</path>",
        "<type>file</type>",
        "<content>1: first",
        "2: second",
        "</content>",
      ].join("\n"),
    }),
    [
      "read: apps/daemon/src/provider/service.rs [offset=480, limit=220]",
      "1: first",
      "2: second",
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate collapses long read output in the middle", () => {
  const content = Array.from({ length: 24 }, (_, index) => `${index + 1}: line ${index + 1}`).join("\n")

  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-5",
      tool: "read",
      status: "completed",
      input: {
        filePath: "apps/cli/src/runtime.ts",
      },
      output: `<path>/Users/miguel/arroba/apps/cli/src/runtime.ts</path>\n<type>file</type>\n<content>${content}\n</content>`,
    }),
    [
      "read: apps/cli/src/runtime.ts",
      ...Array.from({ length: 10 }, (_, index) => `${index + 1}: line ${index + 1}`),
      "...",
      ...Array.from({ length: 10 }, (_, index) => `${index + 15}: line ${index + 15}`),
    ].join("\n"),
  )
})

test("formatToolTranscriptUpdate renders grep output with a compact header", () => {
  assert.equal(
    formatToolTranscriptUpdate({
      id: "tool-6",
      tool: "grep",
      status: "completed",
      input: {
        pattern: "status_updates.push|provider_idle = true|OpenCode is idle|thinking|idle",
        path: "/Users/miguel/arroba",
      },
      output: [
        "Found 13 matches",
        "/Users/miguel/arroba/apps/daemon/src/provider/service.rs:",
        "  Line 416:             status_updates.push(delta)",
        "  Line 418:             provider_idle = true;",
      ].join("\n"),
    }),
    [
      "grep: status_updates.push|provider_idle = true|OpenCode is idle|thinking|idle in apps/daemon/src/provider/service.rs (13 matches)",
      "apps/daemon/src/provider/service.rs",
      "Line 416:             status_updates.push(delta)",
      "Line 418:             provider_idle = true;",
    ].join("\n"),
  )
})

test("mergeToolTranscriptUpdate keeps prior tool details across partial updates", () => {
  const merged = mergeToolTranscriptUpdate(
    {
      id: "tool-1",
      tool: "bash",
      status: "running",
      input: { command: "git status" },
      description: "Shows working tree status",
    },
    {
      id: "tool-1",
      status: "completed",
      output: "On branch main",
    },
  )

  assert.deepEqual(merged, {
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "git status" },
    description: "Shows working tree status",
    output: "On branch main",
  })
  assert.equal(
    formatToolTranscriptUpdate(merged),
    [
      "bash",
      "Shows working tree status",
      "$ git status",
      "On branch main",
    ].join("\n\n"),
  )
})

test("shouldRenderProviderStatus suppresses idle notices only", () => {
  assert.equal(shouldRenderProviderStatus("OpenCode is idle."), false)
  assert.equal(shouldRenderProviderStatus("OpenCode is thinking..."), true)
})

test("splitInlineCodeSpans marks inline code runs", () => {
  assert.deepEqual(splitInlineCodeSpans("Run `git status` and `git diff`."), [
    { text: "Run ", code: false },
    { text: "git status", code: true },
    { text: " and ", code: false },
    { text: "git diff", code: true },
    { text: ".", code: false },
  ])
})

test("splitInlineCodeSpans leaves unmatched backticks as plain text", () => {
  assert.deepEqual(splitInlineCodeSpans("Use `unfinished inline code"), [
    { text: "Use `unfinished inline code", code: false },
  ])
})

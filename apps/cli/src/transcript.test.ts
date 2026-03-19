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

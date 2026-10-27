import assert from "node:assert/strict"
import test from "node:test"

import {
  formatPromptSubmissionBody,
  formatPromptSubmissionStatusLine,
  promptSubmissionAttachmentsToParts,
} from "@chariox/kernel-client/prompt-submission"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"

test("formatPromptSubmissionBody terminates non-empty prompts", () => {
  assert.equal(formatPromptSubmissionBody("hello"), "hello\n")
  assert.equal(formatPromptSubmissionBody("hello\n"), "hello\n")
  assert.equal(formatPromptSubmissionBody("  "), "")
})

test("promptSubmissionAttachmentsToParts strips prompt-only attachment fields", () => {
  const attachments: PendingPromptAttachment[] = [
    {
      id: "attachment-1",
      url: "file:///tmp/a.txt",
      mime: "text/plain",
      filename: "a.txt",
      kind: "text",
      token: "[file 1]",
    },
  ]

  assert.deepEqual(promptSubmissionAttachmentsToParts(attachments), [
    {
      url: "file:///tmp/a.txt",
      mime: "text/plain",
      filename: "a.txt",
    },
  ])
})

test("formatPromptSubmissionStatusLine describes queued and submitted outcomes", () => {
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Queued",
      activePromptId: "prompt-1",
    }),
    "Prompt queued behind prompt-1.",
  )
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Queued",
      activePromptId: null,
    }),
    "Prompt queued behind the active turn.",
  )
  assert.equal(
    formatPromptSubmissionStatusLine({
      outcomeName: "Submitted",
      activePromptId: "prompt-1",
    }),
    "Prompt submitted.",
  )
})

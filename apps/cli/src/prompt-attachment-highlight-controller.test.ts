import assert from "node:assert/strict"
import test from "node:test"

import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import {
  createPromptAttachmentHighlightController,
} from "./prompt-attachment-highlight-controller.js"

const attachment = (
  id: string,
  kind: PendingPromptAttachment["kind"],
  token: string,
): PendingPromptAttachment => ({
  id,
  kind,
  token,
  url: `file:///tmp/${id}`,
  mime: "text/plain",
  filename: id,
})

test("refresh highlights every rendered attachment token occurrence", () => {
  const ranges: Array<{ start: number; end: number; styleId: number }> = []
  const input = {
    plainText: "use [file 1] and [image 1] then [file 1]",
    clearAllHighlights: () => {
      ranges.length = 0
    },
    addHighlightByCharRange: (range: { start: number; end: number; styleId: number }) => {
      ranges.push(range)
    },
  }
  const controller = createPromptAttachmentHighlightController({
    getPromptInput: () => input,
    getPendingAttachments: () => [
      attachment("one", "text", "[file 1]"),
      attachment("two", "image", "[image 1]"),
    ],
    styleIdForKind: (kind) => kind === "image" ? 2 : 1,
  })

  assert.equal(controller.refresh(), true)

  assert.deepEqual(ranges, [
    { start: 4, end: 12, styleId: 1 },
    { start: 32, end: 40, styleId: 1 },
    { start: 17, end: 26, styleId: 2 },
  ])
})

test("refresh clears stale highlights even when tokens are absent", () => {
  let clearCount = 0
  const ranges: Array<{ start: number; end: number; styleId: number }> = [{
    start: 0,
    end: 1,
    styleId: -1,
  }]
  const controller = createPromptAttachmentHighlightController({
    getPromptInput: () => ({
      plainText: "plain text",
      clearAllHighlights: () => {
        clearCount += 1
        ranges.length = 0
      },
      addHighlightByCharRange: (range) => {
        ranges.push(range)
      },
    }),
    getPendingAttachments: () => [attachment("one", "pdf", "[pdf 1]")],
    styleIdForKind: () => 3,
  })

  assert.equal(controller.refresh(), true)

  assert.equal(clearCount, 1)
  assert.deepEqual(ranges, [])
})

test("refresh is idle when prompt input is unavailable", () => {
  const controller = createPromptAttachmentHighlightController({
    getPromptInput: () => null,
    getPendingAttachments: () => [attachment("one", "text", "[file 1]")],
    styleIdForKind: () => 1,
  })

  assert.equal(controller.refresh(), false)
})

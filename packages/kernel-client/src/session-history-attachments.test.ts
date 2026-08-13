import assert from "node:assert/strict"
import test from "node:test"

import {
  cloneSessionHistoryPromptAttachments,
  mergeSessionHistoryPromptAttachments,
} from "./session-history-attachments.js"

test("session history attachment clone preserves values without reusing objects", () => {
  const attachments = [attachment("image-1", { preview_url: "data:image/png;base64,aW1hZ2U=" })]
  const cloned = cloneSessionHistoryPromptAttachments(attachments)

  assert.deepEqual(cloned, attachments)
  assert.notEqual(cloned[0], attachments[0])
})

test("session history attachment merge upgrades matching placeholder attachments", () => {
  const merged = mergeSessionHistoryPromptAttachments([
    attachment("image-1", { preview_url: null }),
  ], [
    attachment("image-1", { preview_url: "data:image/png;base64,aW1hZ2U=" }),
  ])

  assert.deepEqual(merged, [
    attachment("image-1", { preview_url: "data:image/png;base64,aW1hZ2U=" }),
  ])
})

test("session history attachment merge preserves extra chips while upgrading matches", () => {
  const merged = mergeSessionHistoryPromptAttachments([
    attachment("image-1", { preview_url: null }),
    attachment("notes", {
      mime: "text/plain",
      filename: "notes.txt",
      preview_url: null,
    }),
  ], [
    attachment("image-1", { preview_url: "data:image/png;base64,aW1hZ2U=" }),
  ])

  assert.deepEqual(merged, [
    attachment("image-1", { preview_url: "data:image/png;base64,aW1hZ2U=" }),
    attachment("notes", {
      mime: "text/plain",
      filename: "notes.txt",
      preview_url: null,
    }),
  ])
})

test("session history attachment merge chooses richer non-overlapping set", () => {
  assert.deepEqual(mergeSessionHistoryPromptAttachments([
    attachment("placeholder", {
      filename: "attachment",
      preview_url: null,
    }),
  ], [
    attachment("image-1", {
      filename: "Screenshot.png",
      preview_url: "data:image/png;base64,aW1hZ2U=",
    }),
  ]), [
    attachment("image-1", {
      filename: "Screenshot.png",
      preview_url: "data:image/png;base64,aW1hZ2U=",
    }),
  ])
})

function attachment(
  id: string,
  overrides: Partial<{
    mime: string
    filename: string | null
    preview_url: string | null
  }> = {},
) {
  return {
    url: `chariox-terminal://prompt-attachment/attachment-1/${id}`,
    mime: "image/png",
    filename: `${id}.png`,
    preview_url: null,
    ...overrides,
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import {
  addPendingPromptAttachments,
  filterPendingPromptAttachmentsForText,
  nextPromptAttachmentToken,
  removePendingPromptAttachmentToken,
  type PendingPromptAttachment,
  type StoredPromptAttachment,
} from "./prompt-attachment-state.js"

function pending(overrides: Partial<PendingPromptAttachment> = {}): PendingPromptAttachment {
  return {
    id: "id",
    url: "file:///one",
    mime: "text/plain",
    filename: "one.txt",
    kind: "text",
    token: "[file 1]",
    ...overrides,
  }
}

function stored(overrides: Partial<StoredPromptAttachment> = {}): StoredPromptAttachment {
  return {
    id: "id",
    url: "file:///one",
    mime: "text/plain",
    filename: "one.txt",
    kind: "text",
    ...overrides,
  }
}

test("nextPromptAttachmentToken counts by rendered token kind", () => {
  assert.equal(nextPromptAttachmentToken([
    pending({ kind: "image", token: "[image 1]" }),
    pending({ kind: "text", token: "[file 1]" }),
    pending({ kind: "pdf", token: "[pdf 1]" }),
  ], "text"), "[file 2]")
  assert.equal(nextPromptAttachmentToken([
    pending({ kind: "image", token: "[image 1]" }),
  ], "image"), "[image 2]")
})

test("filterPendingPromptAttachmentsForText removes attachments whose token was deleted", () => {
  const attachments = [
    pending({ token: "[file 1]" }),
    pending({ id: "two", url: "file:///two", token: "[image 1]", kind: "image" }),
  ]

  assert.deepEqual(filterPendingPromptAttachmentsForText(attachments, "Review [image 1]"), [attachments[1]])
})

test("addPendingPromptAttachments skips duplicate urls and assigns sequential tokens", () => {
  const current = [
    pending({ url: "file:///existing", token: "[file 1]" }),
    pending({ id: "image", url: "file:///image", kind: "image", token: "[image 1]" }),
  ]
  const result = addPendingPromptAttachments(current, [
    stored({ id: "dup", url: "file:///existing", filename: "existing.txt" }),
    stored({ id: "two", url: "file:///two", filename: "two.txt" }),
    stored({ id: "img2", url: "file:///img2", filename: "two.png", mime: "image/png", kind: "image" }),
  ])

  assert.deepEqual(result.addedAttachments.map((file) => file.token), ["[file 2]", "[image 2]"])
  assert.deepEqual(result.nextAttachments.map((file) => file.url), [
    "file:///existing",
    "file:///image",
    "file:///two",
    "file:///img2",
  ])
})

test("removePendingPromptAttachmentToken removes adjacent spacing when token is in text", () => {
  const attachments = [
    pending({ token: "[file 1]" }),
    pending({ id: "two", url: "file:///two", token: "[file 2]" }),
  ]

  assert.deepEqual(removePendingPromptAttachmentToken({
    attachments,
    text: "Review [file 1] now",
    token: "[file 1]",
  }), {
    nextAttachments: [attachments[1]],
    nextText: "Review now",
    cursorOffset: "Review".length,
    tokenFound: true,
  })
})

test("removePendingPromptAttachmentToken still clears pending state when token is absent from text", () => {
  const attachment = pending({ token: "[file 1]" })
  assert.deepEqual(removePendingPromptAttachmentToken({
    attachments: [attachment],
    text: "Review notes",
    token: "[file 1]",
  }), {
    nextAttachments: [],
    nextText: "Review notes",
    cursorOffset: null,
    tokenFound: false,
  })
})

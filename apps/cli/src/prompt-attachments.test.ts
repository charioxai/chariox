import assert from "node:assert/strict"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  classifyPromptAttachment,
  extractDroppedPromptAttachments,
  formatPromptAttachmentSummary,
  parsePromptAttachmentCommand,
  resolvePromptAttachmentEdit,
} from "./prompt-attachments.js"

test("extractDroppedPromptAttachments parses dropped quoted files", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "arroba-cli-attachments-"))
  const alpha = path.join(dir, "notes.md")
  const beta = path.join(dir, "diagram.png")
  fs.writeFileSync(alpha, "hello")
  fs.writeFileSync(beta, "png")

  const result = extractDroppedPromptAttachments(
    "please review\n",
    `please review\n"${alpha}" ${beta} `,
    dir,
  )

  assert.deepEqual(result?.nextText, "please review\n")
  assert.equal(result?.insertAt, "please review\n".length)
  assert.deepEqual(
    result?.files.map((file) => [file.filename, file.kind]),
    [["notes.md", "text"], ["diagram.png", "image"]],
  )
})

test("extractDroppedPromptAttachments preserves insertion point in the middle", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "arroba-cli-attachments-"))
  const alpha = path.join(dir, "diagram.png")
  fs.writeFileSync(alpha, "png")

  const result = extractDroppedPromptAttachments(
    "hello world",
    `hello "${alpha}" world`,
    dir,
  )

  assert.equal(result?.insertAt, 6)
  assert.deepEqual(result?.files.map((file) => file.filename), ["diagram.png"])
})

test("parsePromptAttachmentCommand supports escaped spaces", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "arroba-cli-attachments-"))
  const file = path.join(dir, "my note.txt")
  fs.writeFileSync(file, "hello")

  const result = parsePromptAttachmentCommand(file.replace(/ /g, "\\ "), dir)
  assert.equal(result?.[0]?.filename, "my note.txt")
  assert.equal(result?.[0]?.mime, "text/plain")
})

test("classifyPromptAttachment rejects unsupported binaries", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "arroba-cli-attachments-"))
  const file = path.join(dir, "archive.zip")
  fs.writeFileSync(file, "zip")
  assert.equal(classifyPromptAttachment(file), null)
})

test("formatPromptAttachmentSummary renders compact chips", () => {
  assert.equal(
    formatPromptAttachmentSummary([
      { filename: "notes.md", kind: "text" },
      { filename: "diagram.png", kind: "image" },
    ]),
    "[TXT notes.md] [IMG diagram.png]",
  )
})

test("resolvePromptAttachmentEdit keeps delete after a token on the adjacent character", () => {
  const text = "Review [file 1] now"
  const result = resolvePromptAttachmentEdit(text, ["[file 1]"], "delete", "Review [file 1]".length - 1)

  assert.deepEqual(result, {
    kind: "delete-text",
    start: "Review [file 1]".length,
    end: "Review [file 1]".length + 1,
  })
})

test("resolvePromptAttachmentEdit removes the token when delete happens inside it", () => {
  const text = "Review [file 1] now"
  const result = resolvePromptAttachmentEdit(text, ["[file 1]"], "delete", text.indexOf("[file 1]") + 2)

  assert.deepEqual(result, {
    kind: "remove-attachments",
    start: text.indexOf("[file 1]"),
    end: text.indexOf("[file 1]") + "[file 1]".length,
    tokens: ["[file 1]"],
  })
})

test("resolvePromptAttachmentEdit swallows delete at the end of a token", () => {
  const text = "Review [file 1]"
  const result = resolvePromptAttachmentEdit(text, ["[file 1]"], "delete", text.length - 1)

  assert.deepEqual(result, { kind: "noop" })
})

test("resolvePromptAttachmentEdit deletes selected plain text when no attachment tokens are selected", () => {
  const text = "Review this prompt"
  const result = resolvePromptAttachmentEdit(text, [], "delete", text.length, {
    start: 11,
    end: 4,
  })

  assert.deepEqual(result, {
    kind: "delete-text",
    start: 4,
    end: 11,
  })
})

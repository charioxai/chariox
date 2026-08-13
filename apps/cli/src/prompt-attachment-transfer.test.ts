import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { pathToFileURL } from "node:url"

import {
  localAttachmentPath,
  preparePromptAttachmentsForSubmit,
} from "./prompt-attachment-transfer.js"
import type { PromptAttachmentPart } from "./cli-types.js"

test("preparePromptAttachmentsForSubmit keeps local fast path when inline disabled", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "chariox-attachment-transfer-"))
  try {
    const file = path.join(dir, "note.txt")
    await writeFile(file, "hello")
    const attachments = [{ url: file, mime: "text/plain", filename: "note.txt" }]

    assert.deepEqual(await preparePromptAttachmentsForSubmit(attachments, { inlineLocalFiles: false }), attachments)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("preparePromptAttachmentsForSubmit inlines absolute paths and file URLs", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "chariox-attachment-transfer-"))
  try {
    const first = path.join(dir, "one.txt")
    const second = path.join(dir, "two words.txt")
    await writeFile(first, "one")
    await writeFile(second, "two")

    const input: PromptAttachmentPart[] = [
      { url: first, mime: "text/plain", filename: "one.txt" },
      { url: pathToFileURL(second).toString(), mime: "text/plain", filename: "two words.txt" },
      { url: "https://example.com/remote.png", mime: "image/png", filename: "remote.png" },
    ]
    const attachments = await preparePromptAttachmentsForSubmit(input, { inlineLocalFiles: true })

    assert.equal(attachments[0]?.contents_base64, Buffer.from("one").toString("base64"))
    assert.equal(attachments[1]?.contents_base64, Buffer.from("two").toString("base64"))
    assert.equal(attachments[2]?.contents_base64, undefined)
    assert.equal(await readFile(first, "utf8"), "one")
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("localAttachmentPath resolves file URLs and rejects non-local URLs", () => {
  assert.equal(localAttachmentPath("/tmp/a.txt"), "/tmp/a.txt")
  assert.equal(localAttachmentPath("file:///tmp/a%20b.txt"), "/tmp/a b.txt")
  assert.equal(localAttachmentPath("https://example.com/a.txt"), null)
})

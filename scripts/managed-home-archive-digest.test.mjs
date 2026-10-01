import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { closeSync, fstatSync, openSync } from "node:fs"
import { mkdtemp, rename, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { PassThrough, Readable } from "node:stream"
import { setTimeout as delay } from "node:timers/promises"
import { test } from "node:test"
import { digestHomeArchiveStream, digestPinnedHomeArchive } from "../apps/kernel/slice-linux-docker/managed-home-archive-digest.mjs"

const sha256 = bytes => createHash("sha256").update(bytes).digest("hex")

test("archive digest permits healthy progress beyond the inactivity interval", async () => {
  const chunks = Array.from({ length: 8 }, (_, index) => Buffer.alloc(8192, index))
  const stream = Readable.from((async function* () {
    for (const chunk of chunks) { await delay(20); yield chunk }
  })())
  assert.equal(await digestHomeArchiveStream(stream, 100), sha256(Buffer.concat(chunks)))
})

for (const prefix of [false, true]) {
  test(`archive digest settles a stalled read ${prefix ? "after progress" : "before its first byte"}`, { timeout: 2000 }, async () => {
    const stream = new PassThrough()
    if (prefix) stream.write(Buffer.from("synthetic archive prefix"))
    await assert.rejects(digestHomeArchiveStream(stream, 30), /made no progress/)
    assert.equal(stream.destroyed, true)
  })
}

test("archive digest propagates read failure", async () => {
  const stream = new PassThrough()
  const result = digestHomeArchiveStream(stream, 100)
  stream.destroy(new Error("synthetic read failure"))
  await assert.rejects(result, /synthetic read failure/)
})

test("archive digest rejects invalid inactivity policies", async () => {
  for (const value of [0, -1, null, Infinity, 1.5, 2_147_483_648]) {
    const stream = new PassThrough()
    await assert.rejects(digestHomeArchiveStream(stream, value), /invalid home archive progress timeout/)
    stream.destroy()
  }
})

test("managed digest retains the pinned inode and leaves its caller descriptor open", {
  skip: process.platform !== "linux" && "managed broker requires Linux proc descriptors",
}, async context => {
  const root = await mkdtemp(join(tmpdir(), "chariox-archive-digest-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const path = join(root, "archive")
  const contents = Buffer.alloc(160 * 1024, 173)
  await writeFile(path, contents, { mode: 0o600 })
  const fd = openSync(path, "r")
  context.after(() => closeSync(fd))
  const before = fstatSync(fd)
  await rename(path, join(root, "retained"))
  await writeFile(path, "replacement")
  assert.equal(await digestPinnedHomeArchive(fd, 1000), sha256(contents))
  const after = fstatSync(fd)
  assert.equal(after.ino, before.ino)
  assert.equal(after.size, contents.length)
  assert.equal(after.mode & 0o777, 0o600)
})

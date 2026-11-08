import assert from "node:assert/strict"
import test from "node:test"
import { createHash } from "node:crypto"
import { mkdir, mkdtemp, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { readCaptureInput, writeCaptureOutput } from "./path1-capture-output.mjs"

test("external evidence output is private, exclusive, and rejects symlink and repository paths", async (t) => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-path1-provider-capture-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repository = join(root, "repo")
  const evidence = join(root, "evidence")
  await mkdir(repository)
  await mkdir(evidence)
  const output = join(evidence, "capture.json")
  const capture = { schema: "fixture" }

  await assert.rejects(writeCaptureOutput(join(repository, "capture.json"), capture, repository), /outside the repository/)
  await writeCaptureOutput(output, capture, repository)
  assert.equal((await stat(output)).mode & 0o777, 0o600)
  await assert.rejects(writeCaptureOutput(output, capture, repository), /new file/)

  const fileLink = join(evidence, "capture-link.json")
  await symlink(output, fileLink)
  await assert.rejects(writeCaptureOutput(fileLink, capture, repository), /new file/)
  const parentLink = join(evidence, "repo-alias")
  await symlink(repository, parentLink)
  await assert.rejects(writeCaptureOutput(join(parentLink, "capture.json"), capture, repository), /symlink/)
})

test("capture inputs bind exact retained bytes and reject symlink and oversized files", async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-path1-capture-input-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const file = join(root, "capture.json")
  const bytes = Buffer.from('{"schema":"example"}\n')
  await writeFile(file, bytes)
  const result = await readCaptureInput(file)
  assert.equal(result.capture.schema, "example")
  assert.equal(result.evidence.path, file)
  assert.equal(result.evidence.sha256, `sha256:${createHash("sha256").update(bytes).digest("hex")}`)
  await symlink(file, join(root, "alias.json"))
  await assert.rejects(readCaptureInput(join(root, "alias.json")), /bounded regular/)
  await writeFile(file, Buffer.alloc(1024 * 1024 + 1))
  await assert.rejects(readCaptureInput(file), /bounded regular/)
})

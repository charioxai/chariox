import assert from "node:assert/strict"
import test from "node:test"
import { mkdir, mkdtemp, realpath, rm, stat, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { writeCaptureOutput } from "./path1-capture-output.mjs"

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

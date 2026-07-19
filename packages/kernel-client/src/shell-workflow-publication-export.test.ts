import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { writeWorkflowPublicationExportPackage } from "./shell-workflow-publication-export.js"

test("workflow publication export materializes nested files and executable modes", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-materialize-"))
  try {
    const paths = await writeWorkflowPublicationExportPackage(root, [
      {
        path: "public/index.html",
        content_base64: Buffer.from("<h1>hello</h1>\n").toString("base64"),
      },
      {
        path: "run.sh",
        content_base64: Buffer.from("#!/bin/sh\nexit 0\n").toString("base64"),
        executable: true,
      },
    ])

    assert.deepEqual(paths, [join(root, "public/index.html"), join(root, "run.sh")])
    assert.equal(await readFile(join(root, "public/index.html"), "utf8"), "<h1>hello</h1>\n")
    assert.notEqual((await stat(join(root, "run.sh"))).mode & 0o111, 0)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("workflow publication export rejects unsafe, duplicate, and malformed entries", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-materialize-invalid-"))
  try {
    await assert.rejects(
      writeWorkflowPublicationExportPackage(root, [{ path: "../escape", content_base64: "YQ==" }]),
      /unsafe path/,
    )
    await assert.rejects(
      writeWorkflowPublicationExportPackage(root, [
        { path: "same.txt", content_base64: "YQ==" },
        { path: "same.txt", content_base64: "Yg==" },
      ]),
      /duplicate path/,
    )
    await assert.rejects(
      writeWorkflowPublicationExportPackage(root, [{ path: "invalid.txt", content_base64: "not-base64" }]),
      /not valid base64/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

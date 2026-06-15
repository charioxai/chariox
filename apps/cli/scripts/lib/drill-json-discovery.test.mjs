import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"

test("discovers JSON artifacts by schema and prunes broad roots", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-json-discovery-"))
  try {
    const first = path.join(root, "reports", "one.json")
    const second = path.join(root, ".artifacts", "two.json")
    const ignored = path.join(root, "node_modules", "package", "ignored.json")
    await writeJson(first, { schema: "example.schema.v1" })
    await writeJson(second, { schema: "example.schema.v1" })
    await writeJson(ignored, { schema: "example.schema.v1" })
    await writeJson(path.join(root, "reports", "other.json"), { schema: "other.schema.v1" })
    await writeFileWithDir(path.join(root, "reports", "broken.json"), "{not json}\n")
    await writeFileWithDir(path.join(root, "reports", "text.txt"), JSON.stringify({ schema: "example.schema.v1" }))
    const relativeRoot = path.relative(process.cwd(), root)
    const relativeFile = path.relative(process.cwd(), first)

    assert.deepEqual(await findDrillJsonArtifactPaths(root, { schema: "example.schema.v1" }), [second, first].sort())
    assert.deepEqual(await findDrillJsonArtifactPaths(first, { schema: "example.schema.v1" }), [first])
    assert.deepEqual(await findDrillJsonArtifactPaths(relativeRoot, { schema: "example.schema.v1" }), [second, first].sort())
    assert.deepEqual(await findDrillJsonArtifactPaths(relativeFile, { schema: "example.schema.v1" }), [first])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("discovers named artifacts without requiring valid JSON", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-json-discovery-"))
  try {
    const valid = path.join(root, "run-one", "arroba-drill-failure.json")
    const malformed = path.join(root, "run-two", "arroba-drill-failure.json")
    const wrongName = path.join(root, "run-three", "other.json")
    await writeJson(valid, { schema: "arroba.drill.failure.v1" })
    await writeFileWithDir(malformed, "{not json}\n")
    await writeJson(wrongName, { schema: "arroba.drill.failure.v1" })

    assert.deepEqual(await findDrillJsonArtifactPaths(root, {
      fileName: "arroba-drill-failure.json",
    }), [valid, malformed].sort())
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("respects max depth while still reading files at the maximum depth", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-json-discovery-"))
  try {
    const shallow = path.join(root, "one", "report.json")
    const deep = path.join(root, "one", "two", "report.json")
    await writeJson(shallow, { schema: "example.schema.v1" })
    await writeJson(deep, { schema: "example.schema.v1" })

    assert.deepEqual(await findDrillJsonArtifactPaths(root, {
      maxDepth: 1,
      schema: "example.schema.v1",
    }), [shallow])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

async function writeJson(file, value) {
  await writeFileWithDir(file, `${JSON.stringify(value, null, 2)}\n`)
}

async function writeFileWithDir(file, contents) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, "utf8")
}

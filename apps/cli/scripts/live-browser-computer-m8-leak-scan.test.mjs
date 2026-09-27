import assert from "node:assert/strict"
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import test from "node:test"

import { collectFiles } from "./live-browser-computer-m8-leak-scan.mjs"

const scriptPath = fileURLToPath(new URL("./live-browser-computer-m8-leak-scan.mjs", import.meta.url))
const categories = ["log", "history", "trace", "screenshot-metadata", "clipboard", "helper-output"]

async function fixture(t, { omitCategory = null, canary = "m8-fixture-canary-4d89c1" } = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-m8-leak-scan-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const artifactRoots = {}
  for (const category of categories) {
    if (category === omitCategory) continue
    const categoryRoot = path.join(root, category)
    await mkdir(categoryRoot, { recursive: true })
    artifactRoots[category] = categoryRoot
    await writeFile(path.join(categoryRoot, "evidence.bin"), Buffer.from(`safe-${category}`))
  }
  const canaryFile = path.join(root, "canaries.json")
  await writeFile(canaryFile, JSON.stringify({ canary_values: [canary] }), { mode: 0o600 })
  return { root, artifactRoots, canaryFile, canary }
}

function invoke({ artifactRoots, canaryFile }) {
  const args = [scriptPath, "--canaries-file", canaryFile]
  for (const [category, root] of Object.entries(artifactRoots)) args.push("--artifact-root", `${category}=${root}`)
  return spawnSync(process.execPath, args, { encoding: "utf8", timeout: 10_000 })
}

test("public checker detects a planted screenshot-byte canary without returning its value or path", async (t) => {
  const fixtureData = await fixture(t)
  const screenshot = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    Buffer.from("tEXt\0Comment\0", "binary"),
    Buffer.alloc(65_536 - 5 - Buffer.byteLength("\x89PNG\r\n\x1a\ntEXt\0Comment\0", "binary")),
    Buffer.from(fixtureData.canary, "utf8"),
  ])
  await writeFile(path.join(fixtureData.artifactRoots["screenshot-metadata"], "capture.png"), screenshot)

  const result = invoke(fixtureData)
  assert.ifError(result.error)
  assert.equal(result.status, 1)
  const report = JSON.parse(result.stdout)
  assert.equal(report.status, "fail")
  assert.equal(report.leak_count, 1)
  const category = report.categories.find((entry) => entry.category === "screenshot-metadata")
  assert.equal(category.match_count, 1)
  assert.equal(report.matches.length, 1)
  assert.match(report.matches[0].artifact_id, /^sha256:[0-9a-f]{64}$/)
  assert.match(report.matches[0].canary_id, /^sha256:[0-9a-f]{64}$/)
  assert.equal(result.stdout.includes(fixtureData.canary), false)
  assert.equal(result.stdout.includes(fixtureData.root), false)
  assert.equal(result.stderr, "")
})

test("public checker reports an absent artifact class as incomplete, never clean", async (t) => {
  const fixtureData = await fixture(t, { omitCategory: "helper-output" })
  const result = invoke(fixtureData)
  assert.ifError(result.error)
  assert.equal(result.status, 2)
  const report = JSON.parse(result.stdout)
  assert.equal(report.status, "incomplete")
  assert.equal(report.ok, false)
  const category = report.categories.find((entry) => entry.category === "helper-output")
  assert.equal(category.missing, true)
  assert.equal(category.artifact_count, 0)
  assert.equal(result.stdout.includes(fixtureData.canary), false)
  assert.equal(result.stdout.includes(fixtureData.root), false)
  assert.equal(result.stderr, "")
})

test("public checker passes only when every required artifact class is present and clear", async (t) => {
  const fixtureData = await fixture(t)
  const result = invoke(fixtureData)
  assert.ifError(result.error)
  assert.equal(result.status, 0)
  const report = JSON.parse(result.stdout)
  assert.equal(report.status, "pass")
  assert.equal(report.ok, true)
  assert.equal(report.leak_count, 0)
  assert.equal(report.categories.length, categories.length)
  assert.ok(report.categories.every((entry) => entry.missing === false && entry.artifact_count > 0))
  assert.equal(result.stdout.includes(fixtureData.canary), false)
  assert.equal(result.stdout.includes(fixtureData.root), false)
  assert.equal(result.stderr, "")
})

test("nested artifact directories share one file-count limit", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-m8-file-limit-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(path.join(root, "nested"))
  await writeFile(path.join(root, "first"), "a")
  await writeFile(path.join(root, "nested", "second"), "b")
  await writeFile(path.join(root, "nested", "third"), "c")

  await assert.rejects(collectFiles(root, { maxFiles: 2 }), /unavailable artifact class/)
})

test("a short canary wholly inside the previous stream chunk is counted once", async (t) => {
  const shortCanary = "m8-short-canary-4d89c1"
  const longCanary = "m8-other-longest-canary-5d90b2-extended"
  const fixtureData = await fixture(t, { canary: shortCanary })
  await writeFile(fixtureData.canaryFile, JSON.stringify({ canary_values: [shortCanary, longCanary] }))
  const artifact = Buffer.concat([
    Buffer.alloc(65_536 - Buffer.byteLength(shortCanary), "x"),
    Buffer.from(shortCanary),
    Buffer.from("next chunk"),
  ])
  await writeFile(path.join(fixtureData.artifactRoots.log, "boundary.bin"), artifact)

  const result = invoke(fixtureData)
  assert.ifError(result.error)
  assert.equal(result.status, 1)
  const report = JSON.parse(result.stdout)
  assert.equal(report.leak_count, 1)
  assert.equal(report.matches.length, 1)
  assert.equal(report.matches[0].occurrence_count, 1)
  assert.equal(result.stdout.includes(shortCanary), false)
  assert.equal(result.stdout.includes(longCanary), false)
})

test("a canary split across stream chunks is still detected", async (t) => {
  const fixtureData = await fixture(t)
  const artifact = Buffer.concat([
    Buffer.alloc(65_536 - 3, "x"),
    Buffer.from(fixtureData.canary),
  ])
  await writeFile(path.join(fixtureData.artifactRoots.log, "boundary.bin"), artifact)

  const result = invoke(fixtureData)
  assert.ifError(result.error)
  assert.equal(result.status, 1)
  const report = JSON.parse(result.stdout)
  assert.equal(report.leak_count, 1)
  assert.equal(report.matches.length, 1)
  assert.equal(report.matches[0].occurrence_count, 1)
  assert.equal(result.stdout.includes(fixtureData.canary), false)
})

import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
} from "./drill-artifacts.mjs"
import {
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
  summarizeDrillFailureManifest,
  validateDrillFailureManifest,
} from "./drill-failure-manifest.mjs"

test("reads and summarizes a preserved drill failure directory", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-failure-summary-"))
  await prepareDrillArtifacts(root)

  await finalizeDrillArtifacts({
    rootDir: root,
    passed: false,
    failure: new Error("relay target was stale"),
    metadata: {
      drill: "hosted-cloud-relay",
      provider: "opencode-zen",
      token: "should-not-print",
      nested: { ignored: true },
    },
  })

  const manifest = await readDrillFailureManifest(root)
  const summary = summarizeDrillFailureManifest(manifest, { source: root })
  const text = formatDrillFailureManifestSummary(manifest, { source: root })

  assert.equal(summary.schema, "arroba.drill.failure.v1")
  assert.equal(summary.metadata.drill, "hosted-cloud-relay")
  assert.equal(summary.metadata.provider, "opencode-zen")
  assert.equal(summary.metadata.token, "<redacted>")
  assert.equal(summary.metadata.nested, undefined)
  assert.equal(summary.error.name, "Error")
  assert.equal(summary.error.message, "relay target was stale")
  assert.match(text, /drill failure: hosted-cloud-relay/)
  assert.match(text, /metadata: provider=opencode-zen token=<redacted>/)
  assert.match(text, /error=Error: relay target was stale/)
  assert.doesNotMatch(text, /should-not-print/)

  await rm(root, { recursive: true, force: true })
})

test("reads a manifest file directly", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-failure-file-"))
  const file = path.join(root, "failure.json")
  await writeFile(file, `${JSON.stringify(validManifest({ rootDir: root }))}\n`, "utf8")

  const manifest = await readDrillFailureManifest(file)

  assert.equal(manifest.rootDir, root)
  await rm(root, { recursive: true, force: true })
})

test("rejects malformed failure manifests", () => {
  assert.throws(() => validateDrillFailureManifest({}), /unsupported schema/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    rootDir: "",
  }), /missing rootDir/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: [],
  }), /invalid metadata/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    error: { name: "Error", message: "failed", stack: 1 },
  }), /invalid stack/)
})

function validManifest(overrides = {}) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir: "/tmp/arroba-drill",
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill: "test-drill" },
    error: {
      name: "Error",
      message: "failed",
      stack: null,
    },
    ...overrides,
  }
}

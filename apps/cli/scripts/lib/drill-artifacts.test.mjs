import assert from "node:assert/strict"
import { mkdtemp, readFile, stat } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
} from "./drill-artifacts.mjs"

test("drill artifacts are removed after a passing run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-pass-"))
  await prepareDrillArtifacts(root)

  const result = await finalizeDrillArtifacts({ rootDir: root, passed: true })

  assert.equal(result.preserved, false)
  await assert.rejects(stat(root), /ENOENT/)
})

test("drill artifacts are preserved with a failure manifest after a failed run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-fail-"))
  const events = []
  await prepareDrillArtifacts(root)

  const failure = new Error("relay target was stale with Bearer abcdefghijklmnopqrstuvwxyz")
  failure.stack = "Error: relay target was stale with sk-this-should-not-persist\n    at drill"

  const result = await finalizeDrillArtifacts({
    rootDir: root,
    passed: false,
    failure,
    metadata: {
      drill: "hosted-cloud-relay",
      relayToken: "relay-token-should-not-persist",
      provider: "Bearer abcdefghijklmnopqrstuvwxyz",
      nested: { apiKey: "sk-this-should-not-persist" },
    },
    log: (name, details) => events.push({ name, details }),
  })

  assert.equal(result.preserved, true)
  assert.equal(events[0].name, "preserved-failed-run")
  const manifest = JSON.parse(await readFile(result.manifestPath, "utf8"))
  assert.equal(manifest.schema, "arroba.drill.failure.v1")
  assert.equal(manifest.metadata.drill, "hosted-cloud-relay")
  assert.equal(manifest.metadata.relayToken, "<redacted>")
  assert.equal(manifest.metadata.provider, "<redacted>")
  assert.equal(manifest.metadata.nested.apiKey, "<redacted>")
  assert.equal(manifest.error.message, "relay target was stale with <redacted>")
  assert.match(manifest.error.stack, /Error: relay target was stale with <redacted>/)
  assert.doesNotMatch(JSON.stringify(manifest), /should-not-persist|abcdefghijklmnopqrstuvwxyz/)

  await finalizeDrillArtifacts({ rootDir: root, passed: true })
})

import assert from "node:assert/strict"
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

import { matchingLogRecords, parseLogViewerArgs, writeLogBundle } from "./logs.js"
import type { LogRecord } from "./logging.js"

function writeRecords(logDir: string, fileName: string, records: LogRecord[]) {
  mkdirSync(logDir, { recursive: true })
  writeFileSync(
    path.join(logDir, fileName),
    records.map((record) => JSON.stringify(record)).join("\n") + "\n",
    "utf8",
  )
}

test("logs bundle exports filtered session records with a manifest", () => {
  const root = mkdtempSync(path.join(tmpdir(), "chariox-cli-log-bundle-"))
  const logDir = path.join(root, "logs")
  const bundleDir = path.join(root, "bundle")
  try {
    writeRecords(logDir, "daemon.ndjson", [
      {
        timestamp_ms: 100,
        level: "info",
        process_kind: "daemon",
        pid: 11,
        component: "kernel.session",
        message: "other session",
        session_id: "session-other",
      },
      {
        timestamp_ms: 200,
        level: "warn",
        process_kind: "daemon",
        pid: 11,
        component: "kernel.session",
        message: "target session",
        session_id: "session-1",
        provider_run_id: "run-1",
      },
    ])

    const options = parseLogViewerArgs(["--session", "session-1", "--provider-run", "run-1", "--limit", "20", "--bundle", bundleDir])
    const records = matchingLogRecords(logDir, options)
    const bundle = writeLogBundle(logDir, bundleDir, options, records)

    assert.equal(bundle.recordCount, 1)
    assert.equal(bundle.bundleDir, path.resolve(bundleDir))
    assert.match(readFileSync(path.join(bundleDir, "logs.ndjson"), "utf8"), /target session/)
    assert.doesNotMatch(readFileSync(path.join(bundleDir, "logs.ndjson"), "utf8"), /other session/)

    const manifest = JSON.parse(readFileSync(path.join(bundleDir, "manifest.json"), "utf8")) as Record<string, unknown>
    assert.equal(manifest.schema, "chariox.log_bundle.v1")
    assert.equal(manifest.record_count, 1)
    assert.deepEqual(manifest.files, ["logs.ndjson"])
    assert.deepEqual((manifest.filters as Record<string, unknown>).session_id, "session-1")
    assert.deepEqual((manifest.filters as Record<string, unknown>).provider_run_id, "run-1")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test("logs bundle cannot be combined with follow", () => {
  assert.throws(
    () => parseLogViewerArgs(["--follow", "--bundle", "out"]),
    /logs --bundle cannot be combined with --follow/,
  )
})

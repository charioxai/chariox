import assert from "node:assert/strict"
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

import {
  defaultLogDir,
  formatLogRecord,
  readAllLogRecords,
  readLogTail,
  seedLogOffsets,
  type LogRecord,
} from "./logging.js"

function writeRecords(logDir: string, fileName: string, records: LogRecord[]) {
  mkdirSync(logDir, { recursive: true })
  const filePath = path.join(logDir, fileName)
  writeFileSync(
    filePath,
    records.map((record) => JSON.stringify(record)).join("\n") + "\n",
    "utf8",
  )
  return filePath
}

test("defaultLogDir uses machine-local storage instead of the current workspace", () => {
  const previousLogDir = process.env.CHARIOX_LOG_DIR
  const previousHome = process.env.CHARIOX_HOME
  const charioxHome = mkdtempSync(path.join(tmpdir(), "chariox-cli-home-"))
  try {
    delete process.env.CHARIOX_LOG_DIR
    process.env.CHARIOX_HOME = charioxHome
    assert.equal(defaultLogDir(), path.join(charioxHome, "logs"))
    assert.notEqual(defaultLogDir(), path.join(process.cwd(), ".chariox", "logs"))
  } finally {
    if (previousLogDir === undefined) delete process.env.CHARIOX_LOG_DIR
    else process.env.CHARIOX_LOG_DIR = previousLogDir
    if (previousHome === undefined) delete process.env.CHARIOX_HOME
    else process.env.CHARIOX_HOME = previousHome
    rmSync(charioxHome, { recursive: true, force: true })
  }
})

test("readAllLogRecords sorts NDJSON records across files", () => {
  const logDir = mkdtempSync(path.join(tmpdir(), "chariox-cli-logs-"))
  try {
    writeRecords(logDir, "b.ndjson", [
      {
        timestamp_ms: 200,
        level: "info",
        process_kind: "cli",
        pid: 1,
        component: "cli.app",
        message: "second",
      },
    ])
    writeRecords(logDir, "a.ndjson", [
      {
        timestamp_ms: 100,
        level: "info",
        process_kind: "daemon",
        pid: 2,
        component: "daemon.ipc",
        message: "first",
      },
    ])

    const records = readAllLogRecords(logDir)
    assert.deepEqual(records.map((record) => record.message), ["first", "second"])
  } finally {
    rmSync(logDir, { recursive: true, force: true })
  }
})

test("seedLogOffsets and readLogTail only return appended records", () => {
  const logDir = mkdtempSync(path.join(tmpdir(), "chariox-cli-tail-"))
  try {
    const filePath = writeRecords(logDir, "daemon.ndjson", [
      {
        timestamp_ms: 100,
        level: "info",
        process_kind: "daemon",
        pid: 2,
        component: "daemon.main",
        message: "initial",
      },
    ])

    const offsets = seedLogOffsets(logDir)
    writeFileSync(
      filePath,
      [
        JSON.stringify({
          timestamp_ms: 100,
          level: "info",
          process_kind: "daemon",
          pid: 2,
          component: "daemon.main",
          message: "initial",
        }),
        JSON.stringify({
          timestamp_ms: 200,
          level: "warn",
          process_kind: "daemon",
          pid: 2,
          component: "daemon.main",
          message: "appended",
          session_id: "session-1",
        }),
      ].join("\n") + "\n",
      "utf8",
    )

    const tail = readLogTail(logDir, offsets)
    assert.equal(tail.length, 1)
    assert.equal(tail[0]?.message, "appended")
    assert.match(formatLogRecord(tail[0]!), /session=session-1/)
  } finally {
    rmSync(logDir, { recursive: true, force: true })
  }
})

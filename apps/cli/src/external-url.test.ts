import assert from "node:assert/strict"
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { openExternalUrl } from "./external-url.js"

test("MP-08 / MP-10 OS opener receives the complete URL as one argument", { skip: process.platform === "win32" }, async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "url-opener-"))
  const record = path.join(directory, "opened.json")
  const originalPath = process.env.PATH
  const originalRecord = process.env.CHARIOX_URL_SHIM_RECORD
  const url = "https://www.wikipedia.org/?state=" + "abcdef0123456789".repeat(25)
  const helper = path.join(directory, process.platform === "darwin" ? "open" : "xdg-open")
  writeFileSync(helper, '#!/usr/bin/env node\nrequire("node:fs").writeFileSync(process.env.CHARIOX_URL_SHIM_RECORD, JSON.stringify(process.argv.slice(2)))\n')
  chmodSync(helper, 0o700)
  try {
    process.env.PATH = `${directory}${path.delimiter}${originalPath}`
    process.env.CHARIOX_URL_SHIM_RECORD = record
    assert.equal(await openExternalUrl(url), true)
    for (let i = 0; i < 100 && !existsSync(record); i++) await new Promise(resolve => setTimeout(resolve, 10))
    assert.deepEqual(JSON.parse(readFileSync(record, "utf8")), [url])
  } finally {
    if (originalPath === undefined) delete process.env.PATH; else process.env.PATH = originalPath
    if (originalRecord === undefined) delete process.env.CHARIOX_URL_SHIM_RECORD; else process.env.CHARIOX_URL_SHIM_RECORD = originalRecord
    rmSync(directory, { recursive: true, force: true })
  }
})

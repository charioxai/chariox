import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-browser-download-disk-fault-drill.mjs", import.meta.url))

test("browser download disk dry-run records an exact external Node command", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-download-disk-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.browser_download_disk_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, ["fault.disk-pressure", "cleanup.resources"])
    assert.equal(report.command.name, process.execPath)
    assert(report.command.args.includes("--test-concurrency=1"))
    assert.equal(report.resources.length, 0)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("browser download disk drill rejects repository-local evidence", async () => {
  const reportPath = path.join(path.dirname(scriptPath), "browser-download-disk-report.json")
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath]),
    /evidence must stay outside repositories/,
  )
})

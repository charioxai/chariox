import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const script = fileURLToPath(new URL("./live-slice-display-fault-drill.mjs", import.meta.url))

test("display fault drill dry-run records the exact bounded command outside the repository", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-display-fault-dry-run-"))
  const reportPath = path.join(root, "report.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      script,
      "--dry-run",
      "--image",
      "chariox-slice-linux:test",
      "--report",
      reportPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.slice_display_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.equal(report.source.commit.length, 40)
    assert.match(report.source.probeSha256, /^[a-f0-9]{64}$/)
    assert.equal(report.container.image, "chariox-slice-linux:test")
    assert.deepEqual(report.caseIds, [
      "fault.streamer-crash",
      "fault.browser-crash",
      "cleanup.resources",
    ])
    assert.ok(report.container.args.includes("--read-only"))
    assert.match(stdout, /"status":"dry-run"/)
    assert.match(stdout, /"evidenceRoot":"/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("display fault drill rejects repository-local and relative evidence paths", async () => {
  for (const target of ["relative-report.json", path.join(path.dirname(script), "report.json")]) {
    await assert.rejects(
      execFile(process.execPath, [script, "--dry-run", "--report", target]),
      /report path must be absolute|evidence must stay outside repositories/,
    )
  }
})

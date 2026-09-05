import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-browser-controller-fault-drill.mjs", import.meta.url))

test("controller fault drill dry-run records a serial exact-head command externally", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-fault-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))

    assert.equal(report.schema, "chariox.browser_controller_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, ["fault.controller-crash", "cleanup.resources"])
    assert.equal(report.command.name, "cargo")
    assert(report.command.args.includes("--lib"))
    assert.equal(report.command.env.CARGO_BUILD_JOBS, "1")
    assert(path.isAbsolute(report.command.env.CARGO_TARGET_DIR))
    assert.equal(report.resources.length, 0)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("controller fault drill rejects repository-local reports", async () => {
  const reportPath = path.join(path.dirname(scriptPath), "controller-fault-report.json")
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath]),
    /evidence must stay outside repositories/,
  )
})

import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-queue-saturation-fault-drill.mjs", import.meta.url))

test("queue saturation dry-run records an exact serial relay command externally", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-queue-saturation-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.queue_saturation_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, ["fault.queue-saturation", "cleanup.resources"])
    assert.deepEqual(report.command.args.slice(0, 4), ["test", "-p", "chariox-relay", "--lib"])
    assert.equal(report.command.env.CARGO_BUILD_JOBS, "1")
    assert(path.isAbsolute(report.command.env.CARGO_TARGET_DIR))
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-room-takeover-reconnect-fault-drill.mjs", import.meta.url))

test("Room takeover reconnect dry-run records the exact kernel authority probe", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-takeover-reconnect-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.room_takeover_reconnect_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, [
      "fault.takeover-response-loss",
      "reconnect.command-replay",
      "authority.human-retained",
      "authority.agent-blocked",
      "authority.explicit-release",
      "effect.takeover-exactly-once",
      "cleanup.resources",
    ])
    assert.deepEqual(report.command.args.slice(0, 4), ["test", "-p", "chariox-kernel", "--lib"])
    assert.equal(report.command.env.CARGO_BUILD_JOBS, "1")
    assert(path.isAbsolute(report.command.env.CARGO_TARGET_DIR))
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

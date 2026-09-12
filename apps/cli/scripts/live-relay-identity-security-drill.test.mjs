import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-relay-identity-security-drill.mjs", import.meta.url))

test("relay identity dry-run records the external binary and complete time/isolation contract", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-relay-identity-dry-run-"))
  const reportPath = path.join(root, "report.json")
  const relayBinary = path.join(root, "chariox-relay")
  try {
    await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--relay-binary",
      relayBinary,
      "--report",
      reportPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.relay_identity_security_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, [
      "auth.accepted-token-expires",
      "auth.expired-token-rejected",
      "auth.clock-skew-tolerance",
      "auth.future-issued-token-rejected",
      "auth.jwt-format",
      "auth.identity-binding-rejected",
      "isolation.cross-realm",
      "continuity.healthy-routed-round-trip",
      "cleanup.resources",
    ])
    assert.deepEqual(report.command, {
      name: relayBinary,
      args: [],
    })
    assert.equal(report.evidenceRoot, root)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

import assert from "node:assert/strict"
import { execFile as execFileWithCallback, spawn } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const script = fileURLToPath(new URL("./live-slice-display-fault-drill.mjs", import.meta.url))

async function waitForFile(filePath, timeoutMs = 2_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      await readFile(filePath)
      return
    } catch (error) {
      if (error?.code !== "ENOENT") throw error
      await new Promise((resolve) => setTimeout(resolve, 10))
    }
  }
  throw new Error(`timed out waiting for ${filePath}`)
}

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

test("display fault drill interrupted during its resource probe never starts the heavy workload", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-display-fault-interrupt-"))
  const bin = path.join(root, "bin")
  const readyPath = path.join(root, "resource-probe-started")
  const dockerLog = path.join(root, "docker.log")
  const reportPath = path.join(root, "report.json")
  let child = null
  try {
    await mkdir(bin)
    const stubs = {
      git: 'console.log("0123456789abcdef0123456789abcdef01234567")',
      memory_pressure: 'if (!existsSync(process.env.CHARIOX_TEST_DOCKER_LOG)) { process.once("SIGTERM", () => process.exit(143)); appendFileSync(process.env.CHARIOX_TEST_READY, "memory_pressure\\n"); setInterval(() => {}, 1_000) }',
      df: 'if (!existsSync(process.env.CHARIOX_TEST_DOCKER_LOG)) { process.once("SIGTERM", () => process.exit(143)); appendFileSync(process.env.CHARIOX_TEST_READY, "df\\n"); setInterval(() => {}, 1_000) }',
      docker: 'appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, process.argv.slice(2).join(" ") + "\\n")',
    }
    await Promise.all(Object.entries(stubs).map(async ([name, body]) => {
      const target = path.join(bin, name)
      await writeFile(target, `#!${process.execPath}\nimport { appendFileSync, existsSync } from "node:fs"\n${body}\n`)
      await chmod(target, 0o755)
    }))

    child = spawn(process.execPath, [script, "--image", "chariox-slice-linux:test", "--report", reportPath], {
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        CHARIOX_TEST_READY: readyPath,
        CHARIOX_TEST_DOCKER_LOG: dockerLog,
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    const completion = new Promise((resolve) => child.once("close", (code, signal) => resolve({ code, signal })))
    await waitForFile(readyPath)
    child.kill("SIGTERM")
    const result = await completion
    assert.notEqual(result.code, 0)
    const invocations = await readFile(dockerLog, "utf8")
    assert.doesNotMatch(invocations, /^(image|run)(?: |$)/m)
    assert.match(invocations, /^rm -f /m)
    assert.match(invocations, /^ps -aq /m)
  } finally {
    if (child?.exitCode === null && child?.signalCode === null) child.kill("SIGKILL")
    await rm(root, { recursive: true, force: true })
  }
})

import assert from "node:assert/strict"
import { execFile as execFileWithCallback, spawn } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-browser-controller-fault-drill.mjs", import.meta.url))

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

test("controller fault drill interrupted during its resource probe never starts Cargo", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-fault-interrupt-"))
  const bin = path.join(root, "bin")
  const readyPath = path.join(root, "resource-probe-started")
  const cleanupPath = path.join(root, "cleanup-started")
  const cargoLog = path.join(root, "cargo.log")
  const reportPath = path.join(root, "report.json")
  let child = null
  try {
    await mkdir(bin)
    const stubs = {
      git: 'console.log("0123456789abcdef0123456789abcdef01234567")',
      memory_pressure: 'if (!existsSync(process.env.CHARIOX_TEST_CLEANUP)) { process.once("SIGTERM", () => process.exit(143)); appendFileSync(process.env.CHARIOX_TEST_READY, "memory_pressure\\n"); setInterval(() => {}, 1_000) }',
      df: 'if (!existsSync(process.env.CHARIOX_TEST_CLEANUP)) { process.once("SIGTERM", () => process.exit(143)); appendFileSync(process.env.CHARIOX_TEST_READY, "df\\n"); setInterval(() => {}, 1_000) }',
      cargo: 'appendFileSync(process.env.CHARIOX_TEST_CARGO_LOG, process.argv.slice(2).join(" ") + "\\n")',
      pgrep: 'appendFileSync(process.env.CHARIOX_TEST_CLEANUP, "pgrep\\n")',
      ps: "",
    }
    await Promise.all(Object.entries(stubs).map(async ([name, body]) => {
      const target = path.join(bin, name)
      await writeFile(target, `#!${process.execPath}\nimport { appendFileSync, existsSync } from "node:fs"\n${body}\n`)
      await chmod(target, 0o755)
    }))

    child = spawn(process.execPath, [scriptPath, "--cargo-target", path.join(root, "target"), "--report", reportPath], {
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        CHARIOX_TEST_READY: readyPath,
        CHARIOX_TEST_CLEANUP: cleanupPath,
        CHARIOX_TEST_CARGO_LOG: cargoLog,
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    const completion = new Promise((resolve) => child.once("close", (code, signal) => resolve({ code, signal })))
    await waitForFile(readyPath)
    child.kill("SIGTERM")
    const result = await completion
    assert.notEqual(result.code, 0)
    await assert.rejects(readFile(cargoLog), { code: "ENOENT" })
    assert.match(await readFile(cleanupPath, "utf8"), /^pgrep$/m)
  } finally {
    if (child?.exitCode === null && child?.signalCode === null) child.kill("SIGKILL")
    await rm(root, { recursive: true, force: true })
  }
})

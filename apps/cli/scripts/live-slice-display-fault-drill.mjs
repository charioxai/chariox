#!/usr/bin/env node

import { spawn } from "node:child_process"
import { createHash, randomUUID } from "node:crypto"
import { chmod, mkdir, readFile, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  DISPLAY_FAULT_CASE_IDS,
  buildDisplayFaultDockerArgs,
  validateDisplayFaultProbe,
} from "./lib/slice-display-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const sourceRoot = path.join(repoRoot, "apps", "kernel", "slice-linux-docker")
const probePath = path.join(sourceRoot, "validate-slice-viewer.py")
const defaultImage = process.env.CHARIOX_SLICE_IMAGE ?? "chariox-slice-linux:0.1.0"
const children = new Set()
let interrupted = null

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    interrupted ??= signal
    for (const child of children) child.kill("SIGTERM")
  })
}

function usage() {
  console.log([
    "Usage: node apps/cli/scripts/live-slice-display-fault-drill.mjs [options]",
    "",
    "Runs current slice display sources in one network-disabled, resource-capped container.",
    "The probe injects Selkies and Chromium process death, verifies recovery, and cleans up.",
    "",
    "Options:",
    "  --image IMAGE   Existing Chariox slice image with Selkies dependencies",
    "  --report PATH   Absolute external JSON report path",
    "  --dry-run       Record the exact command without contacting Docker",
    "  --help          Show this help",
  ].join("\n"))
}

function parseArgs(argv) {
  const options = { image: defaultImage, reportPath: null, dryRun: false, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--image") options.image = readValue(argv, index++, arg)
    else if (arg.startsWith("--image=")) options.image = arg.slice("--image=".length)
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function defaultReportPath(now = new Date()) {
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(
    os.homedir(),
    ".codex",
    "evidence",
    "browser-computer-use",
    "slice-display-faults",
    stamp,
    "report.json",
  )
}

function resolveReportPath(configured) {
  const reportPath = configured ?? defaultReportPath()
  if (!path.isAbsolute(reportPath)) throw new Error("report path must be absolute")
  const relative = path.relative(repoRoot, reportPath)
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error("display fault evidence must stay outside repositories")
  }
  return path.normalize(reportPath)
}

function run(command, args, { timeoutMs = 180_000, allowFailure = false } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, env: process.env, stdio: ["ignore", "pipe", "pipe"] })
    children.add(child)
    const stdout = []
    const stderr = []
    let timedOut = false
    const timer = setTimeout(() => {
      timedOut = true
      child.kill("SIGTERM")
      setTimeout(() => child.kill("SIGKILL"), 2_000).unref()
    }, timeoutMs)
    child.stdout.on("data", (chunk) => stdout.push(chunk))
    child.stderr.on("data", (chunk) => stderr.push(chunk))
    child.once("error", (error) => {
      clearTimeout(timer)
      children.delete(child)
      reject(error)
    })
    child.once("close", (code, signal) => {
      clearTimeout(timer)
      children.delete(child)
      const result = {
        code,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      }
      if (allowFailure || code === 0) resolve(result)
      else reject(new Error(`${command} exited with ${timedOut ? "timeout" : signal ?? code}: ${bounded(result.stderr)}`))
    })
  })
}

function bounded(value, limit = 2_000) {
  const text = String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "")
  return text.length <= limit ? text : text.slice(-limit)
}

const wait = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))

function checkInterrupted() {
  if (interrupted) throw new Error(`display fault drill interrupted by ${interrupted}`)
}

async function resourceSnapshot(label, containerName = null) {
  const [memory, disk, stats] = await Promise.all([
    run("memory_pressure", ["-Q"], { timeoutMs: 10_000, allowFailure: true }).catch(() => null),
    run("df", ["-k", "/System/Volumes/Data"], { timeoutMs: 10_000, allowFailure: true }).catch(() => null),
    containerName
      ? run("docker", ["stats", "--no-stream", "--format", "{{json .}}", containerName], {
        timeoutMs: 15_000,
        allowFailure: true,
      }).catch(() => null)
      : null,
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    memoryPressure: memory?.code === 0 ? bounded(memory.stdout, 1_000).trim() : null,
    disk: disk?.code === 0 ? disk.stdout.trim().split("\n").at(-1) : null,
    containerStats: stats?.code === 0 ? bounded(stats.stdout, 1_000).trim() : null,
  }
}

function parseProbe(stdout) {
  const lines = stdout.trim().split("\n").filter(Boolean)
  if (lines.length === 0) throw new Error("display fault probe returned no JSON")
  let value
  try {
    value = JSON.parse(lines.at(-1))
  } catch {
    throw new Error("display fault probe returned invalid JSON")
  }
  return validateDisplayFaultProbe(value)
}

async function writeReport(reportPath, report) {
  await mkdir(path.dirname(reportPath), { recursive: true, mode: 0o700 })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await chmod(reportPath, 0o600)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) return usage()
  const reportPath = resolveReportPath(options.reportPath)
  const evidenceRoot = path.dirname(reportPath)
  const containerName = `chariox-slice-display-fault-${process.pid}-${randomUUID().slice(0, 8)}`
  const containerArgs = buildDisplayFaultDockerArgs({ containerName, image: options.image, sourceRoot })
  const report = {
    schema: "chariox.slice_display_fault_drill.v1",
    startedAt: new Date().toISOString(),
    status: options.dryRun ? "dry-run" : "running",
    caseIds: DISPLAY_FAULT_CASE_IDS,
    source: {
      commit: (await run("git", ["rev-parse", "HEAD"], { timeoutMs: 10_000 })).stdout.trim(),
      probeSha256: createHash("sha256").update(await readFile(probePath)).digest("hex"),
    },
    container: {
      name: containerName,
      image: options.image,
      args: containerArgs,
      limits: { memoryBytes: 1_073_741_824, memorySwapBytes: 1_073_741_824, cpus: 1, pids: 256 },
      network: "none",
    },
    evidenceRoot,
    resources: [],
    cleanup: null,
  }
  await mkdir(evidenceRoot, { recursive: true, mode: 0o700 })
  if (options.dryRun) {
    report.completedAt = new Date().toISOString()
    await writeReport(reportPath, report)
    console.log(JSON.stringify({ status: report.status, evidenceRoot, reportPath }))
    return
  }

  let failure = null
  try {
    checkInterrupted()
    report.resources.push(await resourceSnapshot("before"))
    checkInterrupted()
    const image = await run("docker", ["image", "inspect", "--format", "{{.Id}}", options.image], {
      timeoutMs: 30_000,
    })
    report.container.imageId = image.stdout.trim()
    let probeOutcome = null
    const probeExecution = run("docker", containerArgs).then(
      (result) => { probeOutcome = { result } },
      (error) => { probeOutcome = { error } },
    )
    for (let attempt = 0; attempt < 50 && !probeOutcome; attempt += 1) {
      const state = await run("docker", ["inspect", "--format", "{{.State.Running}}", containerName], {
        timeoutMs: 5_000,
        allowFailure: true,
      }).catch(() => null)
      if (state?.code === 0 && state.stdout.trim() === "true") {
        report.resources.push(await resourceSnapshot("during", containerName))
        break
      }
      await wait(200)
    }
    await probeExecution
    if (probeOutcome?.error) throw probeOutcome.error
    report.probe = parseProbe(probeOutcome.result.stdout)
    report.status = "passed"
  } catch (error) {
    failure = error
    report.status = "failed"
    report.failure = bounded(error instanceof Error ? error.message : error)
  } finally {
    const removal = await run("docker", ["rm", "-f", containerName], {
      timeoutMs: 30_000,
      allowFailure: true,
    }).catch(() => null)
    const remaining = await run("docker", ["ps", "-aq", "--filter", `name=^/${containerName}$`], {
      timeoutMs: 15_000,
      allowFailure: true,
    }).catch(() => null)
    const absent = remaining?.code === 0 && remaining.stdout.trim() === ""
    report.cleanup = {
      removalExitCode: removal?.code ?? null,
      containerAbsent: absent,
    }
    report.resources.push(await resourceSnapshot("after-cleanup"))
    report.completedAt = new Date().toISOString()
    if (!absent && !failure) {
      failure = new Error("display fault drill container cleanup failed")
      report.status = "failed"
      report.failure = failure.message
    }
    await writeReport(reportPath, report)
  }
  console.log(JSON.stringify({ status: report.status, evidenceRoot, reportPath }))
  if (failure) throw failure
}

main().catch((error) => {
  console.error(`[slice-display-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})

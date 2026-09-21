import { spawn } from "node:child_process"
import { chmod, mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

const defaultCargoTarget = path.join(os.homedir(), ".chariox", "dev", "browser-computer-use", "cargo-target")

export async function runLocalRustFaultDrill({
  argv,
  repoRoot,
  name,
  description,
  schema,
  caseIds,
  cargoArgs,
  parseProbe,
  evidenceSubdir,
  processNeedle = null,
}) {
  const options = parseArgs(argv, { name, description })
  if (options.help) return { help: true }
  const reportPath = externalPath(options.reportPath ?? defaultReportPath(evidenceSubdir), repoRoot, `${name} evidence`)
  const cargoTarget = externalPath(options.cargoTarget, repoRoot, "Cargo target")
  const children = new Set()
  let interrupted = null
  const terminateChildren = (signal) => {
    interrupted ??= signal
    for (const child of children) terminateGroup(child, "SIGTERM")
  }
  const signalHandlers = new Map(
    ["SIGINT", "SIGTERM"].map((signal) => [signal, () => terminateChildren(signal)]),
  )
  for (const [signal, handler] of signalHandlers) process.once(signal, handler)

  const run = createRunner({ repoRoot, children })
  const report = {
    schema,
    startedAt: new Date().toISOString(),
    status: options.dryRun ? "dry-run" : "running",
    caseIds,
    source: { commit: (await run("git", ["rev-parse", "HEAD"], { timeoutMs: 10_000 })).stdout.trim() },
    command: {
      name: "cargo",
      args: cargoArgs,
      env: { CARGO_BUILD_JOBS: "1", CARGO_TARGET_DIR: cargoTarget },
    },
    evidenceRoot: path.dirname(reportPath),
    resources: [],
    cleanup: null,
  }
  if (options.dryRun) {
    report.completedAt = new Date().toISOString()
    await writeReport(reportPath, report)
    for (const [signal, handler] of signalHandlers) process.removeListener(signal, handler)
    console.log(JSON.stringify({ status: report.status, reportPath }))
    return report
  }

  let failure = null
  let cargoPid = null
  try {
    if (interrupted) throw new Error(`${name} interrupted by ${interrupted}`)
    report.resources.push(await resourceSnapshot(run, "before"))
    if (interrupted) throw new Error(`${name} interrupted by ${interrupted}`)
    const execution = run("cargo", cargoArgs, {
      env: { ...process.env, CARGO_BUILD_JOBS: "1", CARGO_TARGET_DIR: cargoTarget },
      onSpawn: (child) => { cargoPid = child.pid },
    })
    await new Promise((resolve) => setTimeout(resolve, 100))
    report.resources.push(await resourceSnapshot(run, "during", cargoPid))
    const result = await execution
    report.probe = parseProbe(`${result.stdout}\n${result.stderr}`)
    report.output = { stdoutTail: bounded(result.stdout), stderrTail: bounded(result.stderr) }
    report.status = "passed"
  } catch (error) {
    failure = error
    report.status = "failed"
    report.failure = bounded(error instanceof Error ? error.message : error)
  } finally {
    const remaining = processNeedle ? await matchingProcesses(run, processNeedle) : []
    report.cleanup = { ownedProcessesAbsent: remaining.length === 0, remaining }
    report.resources.push(await resourceSnapshot(run, "after-cleanup"))
    report.completedAt = new Date().toISOString()
    if (remaining.length > 0 && !failure) {
      failure = new Error(`${name} left owned processes running`)
      report.status = "failed"
      report.failure = failure.message
    }
    await writeReport(reportPath, report)
    for (const [signal, handler] of signalHandlers) process.removeListener(signal, handler)
  }
  console.log(JSON.stringify({ status: report.status, reportPath }))
  if (failure) throw failure
  return report
}

function parseArgs(argv, { name, description }) {
  const options = { cargoTarget: defaultCargoTarget, reportPath: null, dryRun: false, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--cargo-target") options.cargoTarget = readValue(argv, index++, arg)
    else if (arg.startsWith("--cargo-target=")) options.cargoTarget = arg.slice("--cargo-target=".length)
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.help) {
    console.log([
      `Usage: node ${name} [options]`,
      "",
      description,
      "",
      "  --cargo-target PATH  Absolute shared Cargo target outside the repository",
      "  --report PATH        Absolute external JSON report path",
      "  --dry-run            Record the exact command without running Cargo",
      "  --help               Show this help",
    ].join("\n"))
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function externalPath(value, repoRoot, label) {
  if (!path.isAbsolute(value)) throw new Error(`${label} must be absolute`)
  const normalized = path.normalize(value)
  const relative = path.relative(repoRoot, normalized)
  const withinRepo = relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))
  if (withinRepo) throw new Error(`${label} must stay outside repositories`)
  return normalized
}

function defaultReportPath(evidenceSubdir, now = new Date()) {
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", evidenceSubdir, stamp, "report.json")
}

function createRunner({ repoRoot, children }) {
  return (command, args, { env = process.env, timeoutMs = 600_000, allowFailure = false, onSpawn } = {}) => (
    new Promise((resolve, reject) => {
      const child = spawn(command, args, {
        cwd: repoRoot,
        env,
        detached: process.platform !== "win32",
        stdio: ["ignore", "pipe", "pipe"],
      })
      children.add(child)
      onSpawn?.(child)
      const stdout = []
      const stderr = []
      let timedOut = false
      const timer = setTimeout(() => {
        timedOut = true
        terminateGroup(child, "SIGTERM")
        setTimeout(() => terminateGroup(child, "SIGKILL"), 2_000).unref()
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
  )
}

function terminateGroup(child, signal) {
  if (!child?.pid || child.exitCode !== null) return
  try {
    process.kill(-child.pid, signal)
  } catch {
    child.kill(signal)
  }
}

async function resourceSnapshot(run, label, childPid = null) {
  const [memory, disk, processState] = await Promise.all([
    run("memory_pressure", ["-Q"], { timeoutMs: 10_000, allowFailure: true }).catch(() => null),
    run("df", ["-k", "/System/Volumes/Data"], { timeoutMs: 10_000, allowFailure: true }).catch(() => null),
    childPid
      ? run("ps", ["-p", String(childPid), "-o", "pid=,rss=,%cpu=,etime="], { timeoutMs: 10_000, allowFailure: true }).catch(() => null)
      : null,
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    memoryPressure: memory?.code === 0 ? bounded(memory.stdout, 1_000).trim() : null,
    disk: disk?.code === 0 ? disk.stdout.trim().split("\n").at(-1) : null,
    childProcess: processState?.code === 0 ? processState.stdout.trim() : null,
  }
}

async function matchingProcesses(run, needle) {
  const result = await run("pgrep", ["-f", needle], { timeoutMs: 10_000, allowFailure: true }).catch(() => null)
  return result?.code === 0 ? result.stdout.trim().split("\n").filter(Boolean) : []
}

async function writeReport(reportPath, report) {
  await mkdir(path.dirname(reportPath), { recursive: true, mode: 0o700 })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await chmod(reportPath, 0o600)
}

export function bounded(value, limit = 4_000) {
  const text = String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "")
  return text.length <= limit ? text : text.slice(-limit)
}

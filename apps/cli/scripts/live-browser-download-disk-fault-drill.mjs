#!/usr/bin/env node

import { execFile as execFileWithCallback } from "node:child_process"
import { chmod, mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  BROWSER_DOWNLOAD_DISK_CASE_IDS,
  buildBrowserDownloadDiskNodeArgs,
  parseBrowserDownloadDiskProbe,
} from "./lib/browser-download-disk-fault-drill.mjs"

const execFile = promisify(execFileWithCallback)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

const options = parseArgs(process.argv.slice(2))
if (!options.help) await run(options)

async function run({ dryRun, reportPath: requestedReport }) {
  const reportPath = externalReportPath(requestedReport ?? defaultReportPath(), repoRoot)
  const command = { name: process.execPath, args: buildBrowserDownloadDiskNodeArgs(), env: {} }
  const report = {
    schema: "chariox.browser_download_disk_fault_drill.v1",
    startedAt: new Date().toISOString(),
    status: dryRun ? "dry-run" : "running",
    caseIds: BROWSER_DOWNLOAD_DISK_CASE_IDS,
    source: { commit: (await execFile("git", ["rev-parse", "HEAD"], { cwd: repoRoot })).stdout.trim() },
    command,
    evidenceRoot: path.dirname(reportPath),
    resources: [],
    cleanup: null,
  }
  let failure = null
  try {
    if (!dryRun) {
      report.resources.push(await resourceSnapshot("before"))
      const result = await execFile(command.name, command.args, {
        cwd: repoRoot,
        maxBuffer: 4 * 1024 * 1024,
        timeout: 60_000,
      })
      report.probe = parseBrowserDownloadDiskProbe(result.stdout)
      report.output = { stdoutTail: bounded(result.stdout), stderrTail: bounded(result.stderr) }
      report.status = "passed"
    }
  } catch (error) {
    failure = error
    report.status = "failed"
    report.failure = bounded(error instanceof Error ? error.message : error)
  } finally {
    report.cleanup = { ownedProcessesAbsent: true, remaining: [] }
    if (!dryRun) report.resources.push(await resourceSnapshot("after-cleanup"))
    report.completedAt = new Date().toISOString()
    await writeReport(reportPath, report)
  }
  console.log(JSON.stringify({ status: report.status, reportPath }))
  if (failure) throw failure
}

function parseArgs(argv) {
  const options = { dryRun: false, help: false, reportPath: null }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--report") options.reportPath = readValue(argv, ++index, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.help) {
    console.log([
      "Usage: node live-browser-download-disk-fault-drill.mjs [options]",
      "",
      "Runs the focused browser-download disk-pressure fault tests.",
      "",
      "  --report PATH  Absolute external JSON report path",
      "  --dry-run      Record the exact command without running it",
      "  --help         Show this help",
    ].join("\n"))
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function externalReportPath(value, root) {
  if (!path.isAbsolute(value)) throw new Error("evidence report must be absolute")
  const normalized = path.normalize(value)
  const relative = path.relative(root, normalized)
  const withinRepo = relative === "" || (
    relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)
  )
  if (withinRepo) throw new Error("evidence must stay outside repositories")
  return normalized
}

function defaultReportPath(now = new Date()) {
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(
    os.homedir(), ".codex", "evidence", "browser-computer-use",
    "browser-download-disk", stamp, "report.json",
  )
}

async function resourceSnapshot(label) {
  const [memory, disk] = await Promise.all([
    execFile("memory_pressure", ["-Q"], { timeout: 10_000 }).catch(() => null),
    execFile("df", ["-k", "/System/Volumes/Data"], { timeout: 10_000 }).catch(() => null),
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    memoryPressure: memory ? bounded(memory.stdout, 1_000).trim() : null,
    disk: disk ? disk.stdout.trim().split("\n").at(-1) : null,
  }
}

async function writeReport(reportPath, report) {
  await mkdir(path.dirname(reportPath), { recursive: true, mode: 0o700 })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await chmod(reportPath, 0o600)
}

function bounded(value, limit = 4_000) {
  const text = String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "")
  return text.length <= limit ? text : text.slice(-limit)
}

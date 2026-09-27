#!/usr/bin/env node

import { lstat, mkdir, open, readFile, realpath, stat } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import {
  COORDINATED_LOAD_UNRUN_GATES,
  parseCoordinatedLoadArgs,
  validateCoordinatedLoadConfig,
} from "./lib/room-coordinated-load-plan.mjs"
import { runRoomCoordinatedLoad } from "./lib/room-coordinated-load-runner.mjs"

const scriptPath = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(path.dirname(scriptPath), "../../..")
const maximumConfigBytes = 128 * 1024

export async function runCoordinatedLoadCli(argv, io = console) {
  let options
  try {
    options = parseCoordinatedLoadArgs(argv)
  } catch (error) {
    io.error(`Invalid Drill H arguments: ${error.message}`)
    return 2
  }
  if (options.mode === "help") {
    io.log(helpText())
    return 0
  }

  try {
    const config = await readConfig(options.configPath, options.mode === "execute")
    const plan = validateCoordinatedLoadConfig(config, { repoRoot })
    if (options.mode === "validate") {
      io.log(JSON.stringify({ status: "config_validated", acceptance: "not_proven", runId: plan.runId,
        approvedSliceCount: plan.approval.approvedMaxHeadedSlices, unrunGates: COORDINATED_LOAD_UNRUN_GATES }))
      return 0
    }

    await verifyExternalDirectories([plan.workspace.path, plan.workspace.worktree])
    const evidenceRoot = await createPrivateEvidenceRoot(plan.evidenceRoot)
    const runDirectory = path.join(evidenceRoot, plan.runId)
    await mkdir(runDirectory, { mode: 0o700 })
    const controller = new AbortController()
    const abort = () => controller.abort()
    const totalTimeout = setTimeout(abort, plan.limits.durationMs + 10 * 60 * 1_000)
    process.once("SIGINT", abort)
    process.once("SIGTERM", abort)

    let report
    try {
      const { createRoomCoordinatedLoadRuntime } = await import("./lib/room-coordinated-load-runtime.mjs")
      const runtime = await createRoomCoordinatedLoadRuntime({ plan, repoRoot, runDirectory, signal: controller.signal })
      report = await runRoomCoordinatedLoad(plan, runtime, { signal: controller.signal })
    } catch {
      report = {
        schema: "chariox.room_coordinated_load.report.v1",
        runId: plan.runId,
        status: "failed",
        acceptance: "not_proven",
        failedStage: "runtime_initialization",
        failureCode: "runtime_initialization_failed",
        timing: { requestedDurationMs: plan.limits.durationMs, sampleCount: 0, observedElapsedMs: 0 },
        approvedSliceCount: plan.approval.approvedMaxHeadedSlices,
        verifiedPreparedSliceCount: 0,
        approvalReference: plan.approval.reference,
        samples: [],
        slowViewer: null,
        workflow: { workflowId: null, workflowRunId: null, observedStatuses: [], acceptedByKernel: false, executionObserved: false },
        cleanup: null,
        unrunGates: COORDINATED_LOAD_UNRUN_GATES,
      }
    } finally {
      clearTimeout(totalTimeout)
      process.removeListener("SIGINT", abort)
      process.removeListener("SIGTERM", abort)
    }

    const reportPath = path.join(runDirectory, "report.json")
    await writePrivateReport(reportPath, report)
    io.log(JSON.stringify({ status: report.status, acceptance: "not_proven", reportPath }))
    return report.status === "measured" ? 0 : 1
  } catch (error) {
    io.error(`Drill H stopped: ${safeErrorMessage(error)}`)
    return 1
  }
}

async function readConfig(configPath, requirePrivate) {
  if (!path.isAbsolute(configPath)) throw new Error("--config must be an absolute path")
  const resolved = path.resolve(configPath)
  if (!isOutsideRepo(resolved)) throw new Error("--config must be outside the repository")
  const fileStat = await lstat(resolved)
  if (fileStat.isSymbolicLink() || !fileStat.isFile() || fileStat.size > maximumConfigBytes) {
    throw new Error("--config must be a bounded regular file, not a symlink")
  }
  if (requirePrivate && (fileStat.mode & 0o077) !== 0) {
    throw new Error("live config must have mode 0600 or stricter")
  }
  const actualPath = await realpath(resolved)
  if (!isOutsideRepo(actualPath)) throw new Error("--config resolves inside the repository")
  let config
  try { config = JSON.parse(await readFile(actualPath, "utf8")) } catch {
    throw new Error("--config is not valid JSON")
  }
  return config
}

async function createPrivateEvidenceRoot(configuredRoot) {
  await mkdir(configuredRoot, { recursive: true, mode: 0o700 })
  const actualRoot = await realpath(configuredRoot)
  if (!isOutsideRepo(actualRoot)) throw new Error("evidenceRoot resolves inside the repository")
  const rootStat = await stat(actualRoot)
  if (!rootStat.isDirectory() || (rootStat.mode & 0o077) !== 0) {
    throw new Error("evidenceRoot must be a private directory with mode 0700 or stricter")
  }
  return actualRoot
}

async function verifyExternalDirectories(paths) {
  for (const candidate of paths) {
    const actualPath = await realpath(candidate)
    if (!isOutsideRepo(actualPath) || !(await stat(actualPath)).isDirectory()) {
      throw new Error("workspace and worktree must resolve to existing directories outside the repository")
    }
  }
}

async function writePrivateReport(reportPath, report) {
  const handle = await open(reportPath, "wx", 0o600)
  try {
    await handle.writeFile(`${JSON.stringify(report, null, 2)}\n`, "utf8")
  } finally {
    await handle.close()
  }
}

function isOutsideRepo(candidate) {
  const relative = path.relative(repoRoot, path.resolve(candidate))
  return relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)
}

function safeErrorMessage(error) {
  return error instanceof Error && /^[A-Za-z0-9 _.'():/-]{1,180}$/.test(error.message)
    ? error.message
    : "configuration or output operation failed"
}

function helpText() {
  return [
    "Usage: node apps/cli/scripts/live-room-coordinated-load-drill.mjs --validate-config --config /absolute/path/config.json",
    "       node apps/cli/scripts/live-room-coordinated-load-drill.mjs --execute-live --config /absolute/path/config.json",
    "",
    "Drill H attaches two Room-scoped Selkies viewers and two normal TUI clients to exact prepared local headed slices,",
    "invokes one configured workflow, samples bounded resource and latency metrics, delays one viewer, and stops only tasks it started.",
    "Execution is opt-in, reads relay credentials only from the named environment variable, and writes mode-0600 evidence outside the repository.",
    "A stalled kernel call uses the existing LocalIpcClient 600-second timeout; the client exposes no per-request abort.",
    "A measured report is not an acceptance verdict. Safe maximum admission, Cloud Web rendering, managed-machine execution, and separate 8h/24h soaks remain gates.",
  ].join("\n")
}

const invokedPath = process.argv[1] ? await realpath(path.resolve(process.argv[1])).catch(() => null) : null
if (invokedPath && pathToFileURL(invokedPath).href === import.meta.url) {
  process.exitCode = await runCoordinatedLoadCli(process.argv.slice(2))
}

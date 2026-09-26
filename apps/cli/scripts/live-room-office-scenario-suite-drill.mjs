#!/usr/bin/env node
import { chmod, lstat, mkdir, open, realpath, rmdir } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL, fileURLToPath } from "node:url"
import {
  listRoomOfficeScenarios,
  preflightRoomOfficeScenarioSuite,
  runRoomOfficeScenarioSuite,
} from "./lib/live-room-office-scenario-suite.mjs"

const scriptPath = fileURLToPath(import.meta.url)
const scriptDir = path.dirname(scriptPath)
const repoRoot = path.resolve(scriptDir, "../../..")
const usage = [
  "Usage: node apps/cli/scripts/live-room-office-scenario-suite-drill.mjs [options]",
  "  --list",
  "  --scenario ID --task ID=TEXT --confirm-external-actions ID (repeat per scenario)",
  "  --kernel-url ws://127.0.0.1:PORT --session ID --agent ID --output-dir ABSOLUTE_PATH --execute",
  "  [--timeout-ms 300000] [--poll-ms 1000]",
  "Task text is retained in kernel history. Never include passwords or secret values; use already-authorized Chariox vault handles.",
].join("\n")

function parseArgs(argv) {
  const options = { scenarios: [], tasks: {}, confirmations: [], timeoutMs: 300_000, pollMs: 1_000 }
  const scalarFlags = new Map([
    ["--kernel-url", "kernelUrl"], ["--session", "sessionId"], ["--agent", "agentId"],
    ["--output-dir", "outputDir"], ["--timeout-ms", "timeoutMs"], ["--poll-ms", "pollMs"],
  ])
  const seen = new Set()
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    if (["--help", "--list", "--execute"].includes(flag)) {
      if (seen.has(flag)) throw new Error("invalid options")
      seen.add(flag)
      if (flag === "--help") options.help = true
      else if (flag === "--list") options.list = true
      else options.execute = true
    }
    else if (flag === "--scenario") options.scenarios.push(requireValue(argv, ++index))
    else if (flag === "--confirm-external-actions") options.confirmations.push(requireValue(argv, ++index))
    else if (flag === "--task") {
      const value = requireValue(argv, ++index)
      const separator = value.indexOf("=")
      if (separator < 1 || separator === value.length - 1) throw new Error("invalid options")
      const id = value.slice(0, separator)
      if (Object.hasOwn(options.tasks, id)) throw new Error("invalid options")
      options.tasks[id] = value.slice(separator + 1)
    } else if (scalarFlags.has(flag)) {
      const key = scalarFlags.get(flag)
      if (seen.has(flag)) throw new Error("invalid options")
      seen.add(flag)
      const value = requireValue(argv, ++index)
      options[key] = key.endsWith("Ms") ? Number(value) : value
    } else throw new Error("invalid options")
  }
  if (options.list && argv.length !== 1) throw new Error("invalid options")
  if (options.help && argv.length !== 1) throw new Error("invalid options")
  if (new Set(options.scenarios).size !== options.scenarios.length
    || new Set(options.confirmations).size !== options.confirmations.length) throw new Error("invalid options")
  return options
}

function requireValue(argv, index) {
  const value = argv[index]
  if (typeof value !== "string" || value.startsWith("--")) throw new Error("invalid options")
  return value
}

function isWithin(parent, candidate) {
  return candidate === parent || candidate.startsWith(`${parent}${path.sep}`)
}

export async function resolveEvidenceDirectory(outputDir, repositoryRoot = repoRoot) {
  if (typeof outputDir !== "string" || !path.isAbsolute(outputDir)) throw new Error("invalid output directory")
  const canonicalRepo = await realpath(repositoryRoot)
  const requested = path.resolve(outputDir)
  const parent = await realpath(path.dirname(requested))
  const canonicalTarget = path.join(parent, path.basename(requested))
  if (isWithin(canonicalRepo, parent) || isWithin(canonicalRepo, canonicalTarget)) throw new Error("repository output denied")
  try {
    await lstat(canonicalTarget)
    throw new Error("output already exists")
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
  }
  return canonicalTarget
}

async function createEvidenceDirectory(outputDir) {
  const resolved = await resolveEvidenceDirectory(outputDir)
  await mkdir(resolved, { mode: 0o700 })
  try {
    await chmod(resolved, 0o700)
    const info = await lstat(resolved)
    if (!info.isDirectory() || (info.mode & 0o777) !== 0o700) throw new Error("private output unavailable")
    return resolved
  } catch (error) {
    await rmdir(resolved).catch(() => {})
    throw error
  }
}

async function writePrivateExclusive(filePath, contents) {
  const handle = await open(filePath, "wx", 0o600)
  try {
    await handle.writeFile(contents)
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function loadKernelClient() {
  const ipcUrl = pathToFileURL(path.join(repoRoot, "packages/kernel-client/dist/ipc.js"))
  const requestsUrl = pathToFileURL(path.join(repoRoot, "packages/kernel-client/dist/ipc-requests.js"))
  const [ipc, requests] = await Promise.all([import(ipcUrl.href), import(requestsUrl.href)])
  return { LocalIpcClient: ipc.LocalIpcClient, requests }
}

function jsonLine(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}

async function main(argv) {
  let options
  try {
    options = parseArgs(argv)
  } catch {
    process.stderr.write("error: invalid options; use --help\n")
    process.exitCode = 2
    return
  }
  if (options.help) {
    process.stdout.write(`${usage}\n`)
    return
  }
  if (options.list) {
    jsonLine({ contract: "chariox.room-office-scenario-suite.v1", status: "catalog", scenarios: listRoomOfficeScenarios() })
    return
  }

  const preflight = preflightRoomOfficeScenarioSuite({
    scenarioIds: options.scenarios,
    tasks: options.tasks,
    confirmedScenarios: options.confirmations,
    execute: options.execute,
    kernelUrl: options.kernelUrl,
    sessionId: options.sessionId,
    agentId: options.agentId,
    timeoutMs: options.timeoutMs,
    pollMs: options.pollMs,
    writeScreenshot: async () => {},
  })
  if (preflight.status !== "ready") {
    jsonLine(preflight)
    process.exitCode = 2
    return
  }

  let evidenceDir
  try {
    evidenceDir = await resolveEvidenceDirectory(options.outputDir)
  } catch {
    jsonLine({
      contract: "chariox.room-office-scenario-suite.v1",
      status: "required_user_action",
      reason: "new_external_evidence_directory_required",
      requiredUserActions: ["choose a nonexistent absolute directory outside the repository with an existing parent"],
    })
    process.exitCode = 2
    return
  }

  let runtime
  try {
    runtime = await loadKernelClient()
  } catch {
    jsonLine({
      contract: "chariox.room-office-scenario-suite.v1", status: "unavailable",
      reason: "kernel_client_build_unavailable", requiredUserActions: ["build the existing kernel-client package before executing"],
    })
    process.exitCode = 3
    return
  }

  let client
  let report
  let clientCleanup = "not_opened"
  let evidenceReady = false
  const cleanup = []
  try {
    evidenceDir = await createEvidenceDirectory(evidenceDir)
    evidenceReady = true
    client = new runtime.LocalIpcClient(options.kernelUrl)
    report = await runRoomOfficeScenarioSuite({
      ...options,
      scenarioIds: options.scenarios,
      confirmedScenarios: options.confirmations,
      client,
      requests: runtime.requests,
      recordCleanup: (entry) => cleanup.push(entry),
      writeScreenshot: ({ fileName, bytes }) => writePrivateExclusive(path.join(evidenceDir, fileName), bytes),
    })
  } catch {
    report = {
      contract: "chariox.room-office-scenario-suite.v1", source: "kernel-room-observation",
      status: "unavailable", reason: "kernel_or_private_evidence_unavailable", scenarios: [],
    }
  } finally {
    try {
      if (client?.close) {
        await client.close()
        clientCleanup = "closed"
      }
    } catch {}
  }
  if (client && clientCleanup !== "closed") clientCleanup = "unconfirmed"
  report.cleanup = {
    ...(report.cleanup ?? {}),
    kernelClient: clientCleanup,
    observerAttachments: cleanup,
    externalSideEffects: "not_automatically_reversible",
  }
  if (clientCleanup === "unconfirmed" && report.status === "observed") report.status = "required_user_action"
  if (!evidenceReady) {
    jsonLine(report)
    process.exitCode = 3
    return
  }
  report.evidenceDirectory = evidenceDir
  try {
    await writePrivateExclusive(path.join(evidenceDir, "capture.json"), `${JSON.stringify(report, null, 2)}\n`)
  } catch {
    jsonLine({ contract: "chariox.room-office-scenario-suite.v1", status: "unavailable", reason: "private_evidence_write_failed" })
    process.exitCode = 3
    return
  }
  jsonLine({ contract: report.contract, status: report.status, evidenceDirectory: evidenceDir, scenarioCount: report.scenarios?.length ?? 0 })
  if (report.status !== "observed") process.exitCode = report.status === "required_user_action" ? 2 : 3
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(scriptPath)) {
  await main(process.argv.slice(2))
}

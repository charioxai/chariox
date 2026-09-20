#!/usr/bin/env node

import { execFile as execFileCallback } from "node:child_process"
import { mkdir, readdir, rm, stat } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { promisify } from "node:util"

import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  assertBrowserComputerEvidencePath,
  assertSecretSafeBrowserComputerEvidence,
  collectBrowserComputerResourceSnapshot,
  evaluateBrowserComputerPreflight,
  evaluateBrowserComputerDockerPreconditions,
  evaluateBrowserComputerResourceCaps,
  normalizeBrowserComputerCaps,
  redactBrowserComputerEvidence,
  runBrowserComputerFaultGuard,
} from "./lib/browser-computer-drill-guard.mjs"
import {
  parseManagedBrowserComputerParityArgs,
  readPrivateManagedParityConfig,
} from "./lib/managed-browser-computer-parity-cli.mjs"
import { runManagedBrowserComputerParityHarness } from "./lib/managed-browser-computer-parity-harness.mjs"

const repoRoot = path.resolve(import.meta.dirname, "../../..")
const execFile = promisify(execFileCallback)

const LIVE_STEP_CHECKPOINTS = Object.freeze({
  "docker.save": { before: "before-docker-save", after: "after-docker-save" },
  "docker.remove": { before: "before-docker-remove", after: "after-docker-remove" },
  "docker.restore": { before: "before-docker-restore", after: "after-docker-restore" },
  "selkies.create": { before: "before-browser-start" },
  "selkies.browser": { before: "during-browser" },
  "selkies.destroy": { before: "after-browser-stop" },
})

export async function runManagedBrowserComputerParityLive({
  config,
  transport,
  evidenceRoot,
  signal = null,
  faultAt = null,
  resourceCaps = null,
  collectResourceSnapshot = null,
  dockerPreconditions = null,
  now = () => new Date(),
  secretValues = [],
} = {}) {
  if (!config || typeof config !== "object" || Array.isArray(config)) {
    throw new Error("managed parity live guard requires a config")
  }
  if (!transport || typeof transport.run !== "function") {
    throw new Error("managed parity live guard requires a transport")
  }
  const resolvedEvidenceRoot = assertBrowserComputerEvidencePath(evidenceRoot, [repoRoot])
  const declaredCaps = resourceCaps
    ?? config.browserComputerGuard?.caps
    ?? config.browserComputerCaps
    ?? config.resourceCaps
  const normalizedCaps = normalizeBrowserComputerCaps(declaredCaps)
  const requestedFaultAt = faultAt ?? config.browserComputerGuard?.faultAt ?? null
  const declaredDockerPreconditions = dockerPreconditions
    ?? config.browserComputerGuard?.dockerPreconditions
    ?? []
  if (!Array.isArray(declaredDockerPreconditions)) {
    throw new Error("managed parity browser/computer Docker preconditions must be an array")
  }

  const samples = []
  let report = null
  let resourcePreflight = null
  let activeCheckpoint = null

  const capture = async (phase) => {
    if (samples.some((sample) => sample.phase === phase)) {
      throw new Error(`managed parity resource sample phase was captured more than once: ${phase}`)
    }
    const sample = await (typeof collectResourceSnapshot === "function"
      ? collectResourceSnapshot({
        phase,
        sampleId: `${config.runId}:${phase}`,
        evidenceRoot: resolvedEvidenceRoot,
        transport,
        now,
      })
      : collectLiveResourceSnapshot({
        phase,
        sampleId: `${config.runId}:${phase}`,
        evidenceRoot: resolvedEvidenceRoot,
        transport,
        now,
      }))
    if (!sample || typeof sample !== "object" || Array.isArray(sample)) {
      throw new Error(`managed parity ${phase} resource sample must be an object`)
    }
    const normalized = redactBrowserComputerEvidence({
      ...sample,
      phase,
      sampleId: sample.sampleId ?? `${config.runId}:${phase}`,
    }, { secretValues })
    assertSecretSafeBrowserComputerEvidence(normalized, { secretValues })
    samples.push(normalized)
    return normalized
  }

  const guardedTransport = {
    ...transport,
    run: async (step, request, options) => {
      const checkpoints = LIVE_STEP_CHECKPOINTS[step]
      if (step === "selkies.browser") await capture("during")
      if (checkpoints?.before && activeCheckpoint) {
        await activeCheckpoint(checkpoints.before, { step })
      }
      const result = await transport.run(step, request, options)
      if (checkpoints?.after && activeCheckpoint) {
        await activeCheckpoint(checkpoints.after, { step })
      }
      return result
    },
  }

  const faultGuard = await runBrowserComputerFaultGuard({
    faultAt: requestedFaultAt,
    now,
    secretValues,
    operation: async ({ checkpoint }) => {
      activeCheckpoint = checkpoint
      try {
        assertConfiguredDockerPreconditions(declaredDockerPreconditions)
        const beforeSample = await capture("before")
        resourcePreflight = evaluateBrowserComputerPreflight(beforeSample, {
          requiredMemoryBytes: config.browserComputerGuard?.preflight?.requiredMemoryBytes
            ?? config.browserComputerGuard?.requiredMemoryBytes
            ?? 0,
          requiredDiskBytes: config.browserComputerGuard?.preflight?.requiredDiskBytes
            ?? config.browserComputerGuard?.requiredDiskBytes
            ?? 0,
          allowExistingHeadedSlices: config.browserComputerGuard?.preflight?.allowExistingHeadedSlices === true,
        })
        if (!resourcePreflight.ok) {
          throw new Error(`managed parity browser/computer resource preflight failed: ${resourcePreflight.violations.join("; ")}`)
        }
        report = await runManagedBrowserComputerParityHarness({
          config,
          transport: guardedTransport,
          signal,
          now,
        })
        if (report.status !== "passed") {
          throw new Error(`managed parity harness failed: ${report.failure?.code ?? "unknown"}`)
        }
      } finally {
        try {
          await capture("after")
        } finally {
          activeCheckpoint = null
        }
      }
    },
    cleanup: async () => {
      if (report?.cleanup?.clean !== true) {
        return {
          clean: false,
          ok: false,
          violations: ["managed parity harness cleanup did not prove a clean inventory"],
        }
      }
      return { clean: true, ok: true }
    },
  })

  const resourceEvaluation = evaluateBrowserComputerResourceCaps(samples, normalizedCaps)
  const failed = faultGuard.status !== "passed"
    || !resourceEvaluation.ok
    || report?.status !== "passed"
  const failure = report?.failure ?? (faultGuard.status !== "passed"
    ? { code: "browser_computer_guard_failed", step: faultGuard.faultAt ?? "guard" }
    : !resourceEvaluation.ok
      ? { code: "browser_computer_resource_cap_failed", step: "resource-caps", violations: resourceEvaluation.violations }
      : { code: "managed_parity_failed", step: "harness" })
  const baseReport = report ?? {
    schema: "chariox.browser_computer.managed_parity.v1",
    runId: config.runId,
    status: "failed",
    failure,
    cleanup: { clean: false, inventory: null },
  }
  const finalReport = redactBrowserComputerEvidence({
    ...baseReport,
    status: failed ? "failed" : baseReport.status,
    ...(failed ? { acceptanceBackend: null, failure } : {}),
    browserComputerGuard: {
      schema: "chariox.browser_computer_m0_guard.v1",
      fault: faultGuard,
      resourcePreflight,
      resourceSamples: samples,
      resourceEvaluation,
      dockerPreconditions: declaredDockerPreconditions,
    },
  }, { secretValues })
  assertSecretSafeBrowserComputerEvidence(finalReport, { secretValues })
  return finalReport
}

function assertConfiguredDockerPreconditions(preconditions) {
  for (const input of preconditions) {
    const result = evaluateBrowserComputerDockerPreconditions(input)
    if (!result.ok) {
      throw new Error(`managed parity Docker preconditions failed: ${result.violations.join("; ")}`)
    }
  }
}

async function collectLiveResourceSnapshot({ phase, sampleId, evidenceRoot, transport, now }) {
  if (typeof transport.collectResourceSnapshot === "function") {
    return transport.collectResourceSnapshot({ phase, sampleId, evidenceRoot, now })
  }
  return collectBrowserComputerResourceSnapshot({
    runCommand: runExternalCommand,
    filesystemPath: evidenceRoot,
    phase,
    sampleId,
    now,
    processCount: async () => countProcessRows((await runExternalCommand("ps", ["-e", "-o", "pid="])).stdout),
    logBytes: () => directoryBytes(evidenceRoot),
  })
}

async function runExternalCommand(command, args) {
  try {
    const result = await execFile(command, args, { encoding: "utf8", maxBuffer: 1_000_000 })
    return { code: 0, stdout: result.stdout ?? "", stderr: result.stderr ?? "" }
  } catch (error) {
    return {
      code: Number.isInteger(error?.code) ? error.code : 1,
      stdout: error?.stdout ?? "",
      stderr: error?.stderr ?? error?.message ?? String(error),
    }
  }
}

function countProcessRows(stdout) {
  return String(stdout).split("\n").map((line) => line.trim()).filter(Boolean).length
}

async function directoryBytes(directory) {
  let total = 0
  let entries
  try {
    entries = await readdir(directory, { withFileTypes: true })
  } catch (error) {
    if (error?.code === "ENOENT") return 0
    throw error
  }
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) total += await directoryBytes(entryPath)
    else if (entry.isFile()) total += (await stat(entryPath)).size
  }
  return total
}

async function main() {
  let incompleteRunDir = null
  let transport = null
  const interruption = new AbortController()
  const interrupt = () => interruption.abort()
  process.once("SIGINT", interrupt)
  process.once("SIGTERM", interrupt)
  try {
    const options = parseManagedBrowserComputerParityArgs(process.argv.slice(2), { repoRoot })
    const config = await readPrivateManagedParityConfig(options.configPath)
    const imported = await import(pathToFileURL(options.transportModulePath).href)
    if (typeof imported.createManagedBrowserComputerParityTransport !== "function") {
      throw new Error("transport module must export createManagedBrowserComputerParityTransport")
    }
    const runDir = path.join(options.evidenceRoot, config.runId)
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(config.runId ?? "")) {
      throw new Error("managed parity run id is invalid")
    }
    incompleteRunDir = runDir
    await mkdir(runDir, { recursive: true, mode: 0o700 })
    transport = await imported.createManagedBrowserComputerParityTransport({
      evidenceRoot: runDir,
      signal: interruption.signal,
    })
    const report = await runManagedBrowserComputerParityLive({
      config,
      transport,
      evidenceRoot: runDir,
      signal: interruption.signal,
    })
    const resultPath = path.join(runDir, "managed-browser-computer-parity.json")
    const artifactIndexPath = path.join(runDir, "chariox-drill-artifacts.json")
    await writeDrillJsonArtifactOutput({
      outputPath: resultPath,
      value: report,
      artifactIndexPath,
    })
    incompleteRunDir = null
    process.stdout.write(`${JSON.stringify({
      schema: report.schema,
      status: report.status,
      runId: report.runId,
      resultPath,
      artifactIndexPath,
      failureCode: report.failure?.code ?? null,
    })}\n`)
    if (report.status !== "passed") process.exitCode = 1
  } catch {
    if (incompleteRunDir) await rm(incompleteRunDir, { recursive: true, force: true }).catch(() => {})
    process.stderr.write("managed browser/computer parity harness failed before producing validated evidence\n")
    process.exitCode = 1
  } finally {
    await transport?.close?.().catch(() => {})
    process.removeListener("SIGINT", interrupt)
    process.removeListener("SIGTERM", interrupt)
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  await main()
}

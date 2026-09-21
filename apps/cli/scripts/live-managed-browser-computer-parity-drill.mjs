#!/usr/bin/env node

import { execFile as execFileCallback } from "node:child_process"
import { mkdir, readdir, rm, stat } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { promisify } from "node:util"

import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  assertBrowserComputerEvidencePath,
  assertBrowserComputerResourceTelemetry,
  assertSecretSafeBrowserComputerEvidence,
  collectBrowserComputerResourceSnapshot,
  evaluateBrowserComputerPreflight,
  evaluateBrowserComputerDockerPreconditions,
  evaluateBrowserComputerPersistenceMutationSeams,
  evaluateBrowserComputerResourceCaps,
  evaluateBrowserComputerResourceWatchdogSample,
  redactBrowserComputerEvidence,
  resolveBrowserComputerCaps,
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
const DEFAULT_WATCHDOG_INTERVAL_MS = 250

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
  const normalizedCaps = resolveBrowserComputerCaps({
    caps: declaredCaps,
    resourceCeilings: config.resourceCeilings,
  })
  const requestedFaultAt = faultAt ?? config.browserComputerGuard?.faultAt ?? null
  const declaredDockerPreconditions = dockerPreconditions
    ?? config.browserComputerGuard?.dockerPreconditions
    ?? []
  if (!Array.isArray(declaredDockerPreconditions)) {
    throw new Error("managed parity browser/computer Docker preconditions must be an array")
  }

  const telemetryPolicy = config.browserComputerGuard?.resourceTelemetry
    ?? config.browserComputerGuard?.telemetry
    ?? {}
  const explicitLocalTransport = transport.resourceScope === "local"
    || transport.transportScope === "local"
    || transport.transportKind === "local"
    || transport.local === true
    || config.transport?.scope === "local"
  const localFallbackRequested = telemetryPolicy.mode === "local"
    || telemetryPolicy.allowLocalFallback === true
    || config.browserComputerGuard?.allowLocalResourceFallback === true
  if (localFallbackRequested && !explicitLocalTransport) {
    throw new Error("local resource telemetry fallback requires an explicitly local transport")
  }
  const allowLocalFallback = localFallbackRequested && explicitLocalTransport
  const watchdogIntervalMs = resolveWatchdogInterval(config.browserComputerGuard?.watchdogIntervalMs)
  const watchdogProbeTimeoutMs = resolveWatchdogProbeTimeout(
    config.browserComputerGuard?.watchdogProbeTimeoutMs
      ?? config.browserComputerGuard?.watchdogCaptureTimeoutMs,
  )
  const expectedTelemetryTargetIds = [
    config.expected?.machineId,
    config.expected?.kernelId,
    config.expected?.targetId,
    config.expected?.targetRef,
    config.expected?.resolvedTargetId,
    config.expected?.resolvedTarget?.id,
    config.expected?.resolvedTarget?.targetId,
    config.expected?.target?.id,
    config.expected?.target?.targetId,
    transport.targetId,
    transport.targetRef,
    transport.resolvedTargetId,
    transport.resolvedTarget?.id,
    transport.resolvedTarget?.targetId,
  ].filter((value) => typeof value === "string" && value.trim())

  const samples = []
  const watchdogSamples = []
  const watchdogSamplingFailures = []
  let report = null
  let resourcePreflight = null
  let activeCheckpoint = null
  let persistenceMutationSeam = null
  let persistenceMutationError = null
  let watchdogError = null
  let watchdogTimer = null
  let watchdogInFlight = null
  const workloadController = new AbortController()
  const forwardAbort = () => workloadController.abort(signal?.reason)
  if (signal?.aborted) forwardAbort()
  else signal?.addEventListener?.("abort", forwardAbort, { once: true })

  const capture = async (phase, { watchdog = false, signal: probeSignal = null } = {}) => {
    if (probeSignal?.aborted) throw probeSignal.reason ?? new Error(`managed parity ${phase} resource probe was cancelled`)
    const bucket = watchdog ? watchdogSamples : samples
    if (!watchdog && bucket.some((sample) => sample.phase === phase)) {
      throw new Error(`managed parity resource sample phase was captured more than once: ${phase}`)
    }
    const sample = await (typeof collectResourceSnapshot === "function"
      ? collectResourceSnapshot({
        phase,
        sampleId: `${config.runId}:${phase}${watchdog ? `:${watchdogSamples.length}` : ""}`,
        evidenceRoot: resolvedEvidenceRoot,
        transport,
        now,
        allowLocalFallback,
        explicitLocalTransport,
        signal: probeSignal,
      })
      : collectLiveResourceSnapshot({
        phase,
        sampleId: `${config.runId}:${phase}${watchdog ? `:${watchdogSamples.length}` : ""}`,
        evidenceRoot: resolvedEvidenceRoot,
        transport,
        now,
        allowLocalFallback,
        explicitLocalTransport,
        signal: probeSignal,
      }))
    if (probeSignal?.aborted) throw probeSignal.reason ?? new Error(`managed parity ${phase} resource probe was cancelled`)
    if (!sample || typeof sample !== "object" || Array.isArray(sample)) {
      throw new Error(`managed parity ${phase} resource sample must be an object`)
    }
    const normalized = redactBrowserComputerEvidence({
      ...sample,
      phase,
      sampleId: sample.sampleId ?? `${config.runId}:${phase}`,
    }, { secretValues })
    assertBrowserComputerResourceTelemetry(normalized, {
      allowLocalFallback,
      expectedTargetIds: expectedTelemetryTargetIds,
    })
    assertSecretSafeBrowserComputerEvidence(normalized, { secretValues })
    bucket.push(normalized)
    return normalized
  }

  const guardedTransport = {
    ...transport,
    run: async (step, request, options = {}) => {
      const checkpoints = LIVE_STEP_CHECKPOINTS[step]
      if (step === "selkies.browser") await capture("during")
      if (checkpoints?.before && activeCheckpoint) {
        await activeCheckpoint(checkpoints.before, { step })
      }
      let persistencePlan = null
      let persistenceEvents = []
      let persistencePlanResult = null
      let onPersistenceMutation = null
      if (step === "selkies.persistence") {
        if (typeof transport.describePersistenceMutations !== "function") {
          throw new Error("managed parity transport must expose a validated persistence mutation plan")
        }
        persistencePlan = await transport.describePersistenceMutations(request)
        persistencePlanResult = evaluateBrowserComputerPersistenceMutationSeams({
          result: persistencePlan,
          dockerPreconditions: declaredDockerPreconditions,
          mode: "plan",
        })
        if (!persistencePlanResult.ok) {
          throw new Error(`managed parity persistence plan failed: ${persistencePlanResult.violations.join("; ")}`)
        }
        onPersistenceMutation = async (event) => {
          const mutation = event?.mutation ?? event
          const phase = event?.phase ?? event?.checkpointPhase
          const expectedIndex = Math.floor(persistenceEvents.length / 2)
          const expected = persistencePlanResult.mutations[expectedIndex]
          if (!expected || mutation?.action !== expected.action) {
            throw new Error("managed parity persistence callback did not identify the planned mutation")
          }
          if (phase !== "before" && phase !== "after") {
            throw new Error("managed parity persistence callback must identify before or after")
          }
          if (phase === "before") {
            if (persistenceEvents.length % 2 !== 0) {
              throw new Error("managed parity persistence before callback was out of order")
            }
            if (!sameArgv(mutation.argv, expected.argv)) {
              throw new Error(`managed parity ${mutation.action} callback argv differs from its validated plan`)
            }
            await activeCheckpoint(expected.checkpoints.before, {
              step,
              action: mutation.action,
              argv: mutation.argv,
            })
          } else {
            if (persistenceEvents.length % 2 !== 1) {
              throw new Error("managed parity persistence after callback was out of order")
            }
            if (!expected || expected.action !== mutation.action) {
              throw new Error("managed parity persistence after callback was out of order")
            }
            await activeCheckpoint(expected.checkpoints.after, {
              step,
              action: mutation.action,
              argv: mutation.argv,
            })
          }
          persistenceEvents.push({ phase, action: mutation.action, argv: mutation.argv })
        }
      }
      let result
      try {
        result = await transport.run(step, request, {
          ...options,
          signal: step.startsWith("cleanup.")
            ? (options.signal ?? null)
            : combineBrowserComputerAbortSignals(options.signal, workloadController.signal),
          ...(onPersistenceMutation ? { onPersistenceMutation } : {}),
        })
      } catch (error) {
        if (step === "selkies.persistence") persistenceMutationError = error
        throw error
      }
      if (step === "selkies.persistence") {
        if (persistenceEvents.length !== persistencePlanResult.mutations.length * 2) {
          throw new Error("managed parity persistence transport did not report every save/remove/restore mutation seam")
        }
        const actual = evaluateBrowserComputerPersistenceMutationSeams({
          result,
          plan: persistencePlan,
          dockerPreconditions: declaredDockerPreconditions,
        })
        if (!actual.ok) {
          const error = new Error(`managed parity persistence mutation evidence failed: ${actual.violations.join("; ")}`)
          persistenceMutationError = error
          throw error
        }
        persistenceMutationSeam = actual
      }
      if (checkpoints?.after && activeCheckpoint) {
        await activeCheckpoint(checkpoints.after, { step })
      }
      return result
    },
  }

  const recordWatchdogSamplingFailure = (error) => {
    const failure = {
      code: error?.code ?? "browser_computer_watchdog_sampling_failed",
      message: error?.message ?? String(error),
    }
    watchdogSamplingFailures.push(failure)
    if (!watchdogError) {
      const samplingError = new Error(`managed parity browser/computer watchdog sampling failed: ${failure.message}`)
      samplingError.code = "browser_computer_watchdog_sampling_failed"
      samplingError.samplingFailure = failure
      watchdogError = samplingError
      workloadController.abort(samplingError)
    }
  }

  const captureWatchdogProbe = async () => {
    const probeController = new AbortController()
    let timeout
    let timeoutError = null
    try {
      const timeoutPromise = new Promise((_, reject) => {
        timeout = setTimeout(() => {
          timeoutError = new Error(`managed parity watchdog resource probe exceeded ${watchdogProbeTimeoutMs}ms`)
          timeoutError.code = "browser_computer_watchdog_sampling_timeout"
          probeController.abort(timeoutError)
          reject(timeoutError)
        }, watchdogProbeTimeoutMs)
      })
      return await Promise.race([
        capture("watchdog", { watchdog: true, signal: probeController.signal }),
        timeoutPromise,
      ])
    } finally {
      clearTimeout(timeout)
      if (!probeController.signal.aborted) probeController.abort(timeoutError ?? new Error("watchdog resource probe complete"))
    }
  }

  const runWatchdogTick = async (beforeSample) => {
    if (watchdogError || workloadController.signal.aborted) return
    if (watchdogInFlight) return watchdogInFlight
    watchdogInFlight = (async () => {
      try {
        const sample = await captureWatchdogProbe()
        const evaluation = evaluateBrowserComputerResourceWatchdogSample(beforeSample, sample, normalizedCaps)
        if (!evaluation.ok) {
          const error = new Error(`managed parity browser/computer watchdog failed: ${evaluation.violations.join("; ")}`)
          error.code = "browser_computer_watchdog_failed"
          error.evaluation = evaluation
          watchdogError = error
          workloadController.abort(error)
        }
      } catch (error) {
        recordWatchdogSamplingFailure(error)
      } finally {
        watchdogInFlight = null
      }
    })()
    return watchdogInFlight
  }

  const startWatchdog = (beforeSample) => {
    watchdogTimer = setInterval(() => {
      void runWatchdogTick(beforeSample)
    }, watchdogIntervalMs)
    watchdogTimer.unref?.()
  }

  const stopWatchdog = async () => {
    if (watchdogTimer) clearInterval(watchdogTimer)
    watchdogTimer = null
    if (watchdogInFlight) await watchdogInFlight
  }

  const faultGuard = await runBrowserComputerFaultGuard({
    faultAt: requestedFaultAt,
    now,
    secretValues,
    operation: async ({ checkpoint }) => {
      activeCheckpoint = checkpoint
      try {
        assertConfiguredDockerPreconditions(declaredDockerPreconditions)
        if (transport.requiresCompatibilityPreflight === true
          && typeof transport.assertCompatibilityPreflight !== "function") {
          throw new Error("managed parity live transport must expose compatibility preflight before telemetry")
        }
        if (typeof transport.assertCompatibilityPreflight === "function") {
          await transport.assertCompatibilityPreflight({
            config,
            signal: workloadController.signal,
          })
        }
        const beforeSample = await capture("before")
        resourcePreflight = evaluateBrowserComputerPreflight(beforeSample, {
          requiredMemoryBytes: config.browserComputerGuard?.preflight?.requiredMemoryBytes
            ?? config.browserComputerGuard?.requiredMemoryBytes
            ?? config.resourceCeilings?.minimumFreeMemoryBytes
            ?? 0,
          requiredDiskBytes: config.browserComputerGuard?.preflight?.requiredDiskBytes
            ?? config.browserComputerGuard?.requiredDiskBytes
            ?? config.resourceCeilings?.minimumFreeDiskBytes
            ?? 0,
          allowExistingHeadedSlices: config.browserComputerGuard?.preflight?.allowExistingHeadedSlices === true,
        })
        if (!resourcePreflight.ok) {
          throw new Error(`managed parity browser/computer resource preflight failed: ${resourcePreflight.violations.join("; ")}`)
        }
        startWatchdog(beforeSample)
        try {
          report = await runManagedBrowserComputerParityHarness({
            config,
            transport: guardedTransport,
            signal: workloadController.signal,
            now,
          })
          if (report.status !== "passed") {
            throw new Error(`managed parity harness failed: ${report.failure?.code ?? "unknown"}`)
          }
          if (watchdogError) throw watchdogError
        } finally {
          await stopWatchdog()
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
      if (!report) return { clean: true, ok: true, skipped: true }
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

  const resourceEvaluation = evaluateBrowserComputerResourceCaps(samples, normalizedCaps, {
    additionalSamples: watchdogSamples,
  })
  const failed = faultGuard.status !== "passed"
    || !resourceEvaluation.ok
    || watchdogError !== null
    || report?.status !== "passed"
  const failure = watchdogError
    ? { code: watchdogError.code ?? "browser_computer_watchdog_failed", step: "watchdog", message: watchdogError.message }
    : report?.failure
      ? {
        ...report.failure,
        ...(persistenceMutationError ? { message: persistenceMutationError.message } : {}),
      }
      : (faultGuard.status !== "passed"
    ? {
      code: "browser_computer_guard_failed",
      step: faultGuard.faultAt ?? "guard",
      message: faultGuard.failure?.message ?? null,
    }
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
      watchdogSamples,
      watchdog: {
        intervalMs: watchdogIntervalMs,
        probeTimeoutMs: watchdogProbeTimeoutMs,
        telemetryMode: samples[0]?.telemetry?.scope ?? null,
        samplingFailures: watchdogSamplingFailures,
        error: watchdogError ? { code: watchdogError.code ?? null, message: watchdogError.message } : null,
      },
      resourceEvaluation,
      dockerPreconditions: declaredDockerPreconditions,
      persistenceMutationSeam,
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

async function collectLiveResourceSnapshot({
  phase,
  sampleId,
  evidenceRoot,
  transport,
  now,
  allowLocalFallback,
  explicitLocalTransport,
  signal,
}) {
  if (typeof transport.collectManagedTargetResourceSnapshot === "function") {
    return transport.collectManagedTargetResourceSnapshot({ phase, sampleId, evidenceRoot, now, signal })
  }
  if (typeof transport.collectResourceSnapshot === "function") {
    return transport.collectResourceSnapshot({ phase, sampleId, evidenceRoot, now, signal })
  }
  if (!allowLocalFallback || !explicitLocalTransport) {
    throw new Error("managed parity remote transport must provide authoritative managed-target resource telemetry")
  }
  const snapshot = await collectBrowserComputerResourceSnapshot({
    runCommand: runExternalCommand,
    filesystemPath: evidenceRoot,
    phase,
    sampleId,
    now,
    processCount: async ({ signal: processSignal } = {}) => countProcessRows((await runExternalCommand("ps", ["-e", "-o", "pid="], { signal: processSignal })).stdout),
    logBytes: ({ signal: logSignal } = {}) => directoryBytes(evidenceRoot, logSignal),
    signal,
  })
  return {
    ...snapshot,
    telemetry: {
      scope: "local-host",
      authoritative: true,
      fallback: true,
      source: "explicit-local-fallback",
    },
  }
}

async function runExternalCommand(command, args, { signal = null } = {}) {
  try {
    const result = await execFile(command, args, { encoding: "utf8", maxBuffer: 1_000_000, signal })
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

async function directoryBytes(directory, signal = null) {
  if (signal?.aborted) throw signal.reason ?? new Error("directory byte probe cancelled")
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
    if (entry.isDirectory()) total += await directoryBytes(entryPath, signal)
    else if (entry.isFile()) total += (await stat(entryPath)).size
  }
  return total
}

function resolveWatchdogInterval(value) {
  if (value === undefined || value === null) return DEFAULT_WATCHDOG_INTERVAL_MS
  const interval = Number(value)
  if (!Number.isSafeInteger(interval) || interval < 1 || interval > 60_000) {
    throw new Error("browser/computer watchdog interval must be a safe integer from 1ms through 60000ms")
  }
  return interval
}

function resolveWatchdogProbeTimeout(value) {
  if (value === undefined || value === null) return 5_000
  const timeout = Number(value)
  if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 60_000) {
    throw new Error("browser/computer watchdog probe timeout must be a safe integer from 1ms through 60000ms")
  }
  return timeout
}

export function combineBrowserComputerAbortSignals(...values) {
  const signals = values.flat().filter((value) => value && typeof value.aborted === "boolean")
  if (signals.length === 0) return null
  if (signals.length === 1) return signals[0]
  if (typeof AbortSignal.any === "function") return AbortSignal.any(signals)
  const controller = new AbortController()
  const abort = (signal) => {
    if (!controller.signal.aborted) controller.abort(signal.reason)
  }
  for (const signal of signals) {
    if (signal.aborted) abort(signal)
    else signal.addEventListener("abort", () => abort(signal), { once: true })
  }
  return controller.signal
}

function sameArgv(left, right) {
  return Array.isArray(left) && Array.isArray(right)
    && left.length === right.length
    && left.every((value, index) => String(value) === String(right[index]))
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
      config,
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

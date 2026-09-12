import path from "node:path"
import { mkdir, readdir, rm } from "node:fs/promises"

export const DEFAULT_IDLE_SOAK_DURATION_SECONDS = 24 * 60 * 60
export const IDLE_SOAK_SMOKE_DURATION_SECONDS = 15
export const IDLE_SOAK_SYNTHETIC_SECRET_MARKER = "CHARIOX_IDLE_SOAK_SYNTHETIC_SECRET_"

export function parseIdleAuthenticatedBrowserSoakArgs(argv, { repoRoot, homeDir }) {
  const values = new Map()
  const flags = new Set()
  const valueFlags = new Set([
    "--duration-seconds", "--health-interval-seconds", "--sample-interval-seconds",
    "--max-cpu-percent", "--max-rss-mb", "--max-processes", "--min-free-disk-mb",
    "--evidence-root", "--run-dir", "--display-number", "--debug-port",
  ])
  const booleanFlags = new Set(["--smoke", "--preflight", "--detach", "--internal-run", "--help", "-h"])
  for (let index = 0; index < argv.length; index += 1) {
    const raw = argv[index]
    const separator = raw.indexOf("=")
    const flag = separator === -1 ? raw : raw.slice(0, separator)
    if (booleanFlags.has(flag)) {
      if (separator !== -1) throw new Error(`${flag} does not accept a value`)
      flags.add(flag)
      continue
    }
    if (!valueFlags.has(flag)) throw new Error(`unknown argument: ${raw}`)
    const value = separator === -1 ? argv[++index] : raw.slice(separator + 1)
    if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
    values.set(flag, value)
  }

  const smoke = flags.has("--smoke")
  if (smoke && values.has("--duration-seconds")) throw new Error("--smoke cannot be combined with --duration-seconds")
  const evidenceRoot = path.resolve(values.get("--evidence-root")
    ?? path.join(homeDir, ".codex", "evidence", "browser-computer-use", "idle-authenticated-browser-soak"))
  assertExternal(evidenceRoot, repoRoot)
  const runDir = values.has("--run-dir") ? path.resolve(values.get("--run-dir")) : null
  if (runDir) {
    assertExternal(runDir, repoRoot)
    const relative = path.relative(evidenceRoot, runDir)
    if (relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
      throw new Error("run-dir must be inside evidence-root")
    }
  }
  const preflight = flags.has("--preflight")
  const detach = flags.has("--detach")
  const internalRun = flags.has("--internal-run")
  if (preflight && detach) throw new Error("--preflight cannot be combined with --detach")
  if (internalRun && (preflight || detach)) throw new Error("--internal-run cannot be combined with --preflight or --detach")
  return {
    mode: preflight ? "preflight" : detach ? "detach" : "run",
    idleAuthenticated: true,
    smoke,
    durationSeconds: smoke ? IDLE_SOAK_SMOKE_DURATION_SECONDS
      : integer(values.get("--duration-seconds") ?? String(DEFAULT_IDLE_SOAK_DURATION_SECONDS), "duration-seconds", 1, 86_400),
    healthIntervalSeconds: smoke ? 2 : integer(values.get("--health-interval-seconds") ?? "30", "health-interval-seconds", 1, 300),
    sampleIntervalSeconds: smoke ? 2 : integer(values.get("--sample-interval-seconds") ?? "30", "sample-interval-seconds", 1, 300),
    maxCpuPercent: integer(values.get("--max-cpu-percent") ?? "300", "max-cpu-percent", 1, 1_600),
    maxRssMb: integer(values.get("--max-rss-mb") ?? "2048", "max-rss-mb", 1, 65_536),
    maxProcesses: integer(values.get("--max-processes") ?? "32", "max-processes", 1, 4_096),
    minFreeDiskMb: integer(values.get("--min-free-disk-mb") ?? "1024", "min-free-disk-mb", 1, 1_048_576),
    evidenceRoot,
    runDir,
    displayNumber: optionalInteger(values.get("--display-number"), "display-number", 60, 199),
    debugPort: optionalInteger(values.get("--debug-port"), "debug-port", 1024, 65_535),
    internalRun,
    help: flags.has("--help") || flags.has("-h"),
  }
}

export function buildIdleSoakPaths(evidenceRoot, runId) {
  const runDir = path.join(evidenceRoot, runId)
  return {
    runDir,
    log: path.join(runDir, "runner.log"),
    pid: path.join(runDir, "runner.pid"),
    status: path.join(runDir, "status.json"),
    result: path.join(runDir, "result.json"),
    samples: path.join(runDir, "resource-samples.jsonl"),
    cleanup: path.join(runDir, "cleanup-ledger.json"),
    failure: path.join(runDir, "failure.json"),
    preflight: path.join(runDir, "preflight.json"),
    ownership: path.join(runDir, "process-ownership.json"),
    profile: path.join(runDir, "profile-marker.json"),
    checkpoints: path.join(runDir, "health-checkpoints.jsonl"),
    detachContract: path.join(runDir, "detach-contract.json"),
  }
}

export function minimumIdleSoakCheckpointCount(durationSeconds, healthIntervalSeconds) {
  if (!Number.isSafeInteger(durationSeconds) || durationSeconds < 1
    || !Number.isSafeInteger(healthIntervalSeconds) || healthIntervalSeconds < 1) {
    throw new Error("idle soak duration and health interval must be positive integers")
  }
  return 3 + Math.floor((durationSeconds * 1_000 - 1) / (healthIntervalSeconds * 1_000))
}

export function assertCheckpointAdvanced(previous, current) {
  for (const key of ["healthChecks", "controllerRequests", "authenticatedSessionChecks", "profileMarkerChecks", "freshAuthenticatedRequests"]) {
    if (!Number.isSafeInteger(current?.[key]) || current[key] <= (previous?.[key] ?? -1)) {
      throw new Error(`${key} did not advance monotonically`)
    }
  }
  return current
}

export function assertResourceCeilings(sample, limits) {
  for (const value of [sample?.owned?.processCount, sample?.owned?.rssBytes, sample?.owned?.cpuPercent,
    sample?.disk?.availableBytes, sample?.disk?.totalBytes, sample?.host?.totalMemoryBytes,
    sample?.host?.freeMemoryBytes, ...(sample?.host?.loadAverage ?? [])]) {
    if (!Number.isFinite(value) || value < 0) throw new Error("resource samples must contain finite non-negative values")
  }
  if (sample.owned.processCount > limits.maxProcesses) throw new Error("owned process ceiling exceeded")
  if (sample.owned.rssBytes > limits.maxRssMb * 1024 ** 2) throw new Error("owned memory ceiling exceeded")
  if (sample.owned.cpuPercent > limits.maxCpuPercent) throw new Error("owned CPU ceiling exceeded")
  if (sample.disk.availableBytes < limits.minFreeDiskMb * 1024 ** 2) throw new Error("free disk floor violated")
  return sample
}

export function assertRetainedEvidenceRedacted(texts, forbiddenValues) {
  if (texts.some(text => String(text).includes(IDLE_SOAK_SYNTHETIC_SECRET_MARKER))) {
    throw new Error("retained evidence contains a synthetic secret marker")
  }
  for (const value of forbiddenValues) {
    if (!value) continue
    if (texts.some(text => String(text).includes(value))) throw new Error("retained evidence leak detected")
  }
  return true
}

export function validateCompletedIdleSoakResult(value) {
  if (!value || value.schema !== "chariox.idle_authenticated_browser_soak.v1" || value.status !== "passed") {
    throw new Error("idle soak result must be passed")
  }
  for (const key of ["healthChecks", "controllerRequests", "authenticatedSessionChecks", "profileMarkerChecks", "freshAuthenticatedRequests"]) {
    if (!Number.isSafeInteger(value.counters?.[key]) || value.counters[key] <= 0) throw new Error("idle soak result lacks health evidence")
  }
  if (!Number.isSafeInteger(value.durationSeconds) || !Number.isSafeInteger(value.healthIntervalSeconds)
    || !Number.isFinite(value.elapsedMonotonicMs) || value.elapsedMonotonicMs < value.durationSeconds * 1_000) {
    throw new Error("idle soak result lacks monotonic duration evidence")
  }
  const required = minimumIdleSoakCheckpointCount(value.durationSeconds, value.healthIntervalSeconds)
  if (value.checkpoints?.required !== required || value.checkpoints?.count < required) {
    throw new Error("idle soak result lacks checkpoint coverage")
  }
  const final = value.checkpoints?.final
  if (final?.label !== "final" || final.controllerReady !== true || !Number.isSafeInteger(final.controllerPid)
    || !Number.isSafeInteger(final.browserPid) || final.browserAlive !== true || final.freshAuthenticatedRequest !== true
    || final.profileMarkerObserved !== true || final.elapsedMonotonicMs < value.durationSeconds * 1_000) {
    throw new Error("idle soak result lacks final health checkpoint")
  }
  if (!Array.isArray(value.checkpoints.entries) || value.checkpoints.entries.length !== value.checkpoints.count
    || value.checkpoints.entries[0]?.label !== "initial"
    || value.checkpoints.entries.filter(entry => entry.label === "restart").length !== 1) {
    throw new Error("idle soak result lacks authoritative checkpoint entries")
  }
  const cadenceMs = value.healthIntervalSeconds * 1_000
  const cadenceToleranceMs = Math.max(1_000, Math.min(10_000, cadenceMs * 0.25))
  for (const entry of value.checkpoints.entries) {
    if (entry.controllerReady !== true || !Number.isSafeInteger(entry.controllerPid) || !/^\d+$/.test(entry.controllerStartTime ?? "")
      || !Number.isSafeInteger(entry.browserPid) || !/^\d+$/.test(entry.browserStartTime ?? "") || entry.browserAlive !== true
      || entry.freshAuthenticatedRequest !== true || entry.profileMarkerObserved !== true) {
      throw new Error("idle soak result has an incomplete authenticated checkpoint")
    }
  }
  const restart = value.checkpoints.entries.find(entry => entry.label === "restart")
  const initial = value.checkpoints.entries[0]
  if (restart.browserPid === initial.browserPid && restart.browserStartTime === initial.browserStartTime) {
    throw new Error("idle soak result lacks a distinct controlled Chromium restart")
  }
  for (let index = 1; index < value.checkpoints.entries.length; index += 1) {
    const gap = value.checkpoints.entries[index].elapsedMonotonicMs - value.checkpoints.entries[index - 1].elapsedMonotonicMs
    if (!Number.isFinite(gap) || gap < 0 || gap > cadenceMs + cadenceToleranceMs) {
      throw new Error("idle soak result violates checkpoint cadence")
    }
  }
  if (!Number.isSafeInteger(value.resources?.sampleCount) || value.resources.sampleCount <= 0
    || value.resources.ceilingsRespected !== true) throw new Error("idle soak result lacks ceiling evidence")
  assertResourceEvidence(value.resources?.baseline, "baseline")
  assertResourceEvidence(value.resources?.final, "final")
  if (!Number.isFinite(value.resources?.peakOwnedRssBytes) || value.resources.peakOwnedRssBytes < 0
    || !Number.isFinite(value.resources?.peakOwnedCpuPercent) || value.resources.peakOwnedCpuPercent < 0) {
    throw new Error("idle soak result lacks finite resource peaks")
  }
  if (!/^[0-9a-f]{40}$/i.test(value.source?.commit ?? "") || !value.source?.branch || value.source?.dirty !== false) {
    throw new Error("idle soak result lacks clean source provenance")
  }
  if (value.image?.available !== true || !value.image?.imageDigest
    || value.image?.sourceCommit !== value.source.commit) throw new Error("idle soak result lacks matching image provenance")
  if (value.profile?.browserObserved !== true || value.profile?.cookiePersistedAfterRestart !== true
    || value.profile?.controlledRestartCompleted !== true || value.profile?.checks < value.checkpoints.count
    || !/^[0-9a-f]{64}$/i.test(value.profile?.markerDigest ?? "")) {
    throw new Error("idle soak result lacks Chromium restart persistence evidence")
  }
  if (value.redaction?.passed !== true) throw new Error("idle soak result lacks redaction evidence")
  if (value.cleanup?.clean !== true || value.cleanup?.remainingPids?.length !== 0
    || value.cleanup?.remainingListeners?.length !== 0 || value.cleanup?.stateRemoved !== true
    || value.cleanup?.debugPortReleased !== true) throw new Error("idle soak result cleanup is not clean")
  return value
}

export async function assertCleanIdleSoakRunDirectory(runDir, { detachedContinuation = false } = {}) {
  await mkdir(runDir, { recursive: true, mode: 0o700 })
  if (detachedContinuation) await rm(path.join(runDir, "failure.json"), { force: true })
  const allowed = detachedContinuation
    ? new Set(["preflight.json", "runner.log", "runner.pid", "status.json", "detach-contract.json"])
    : new Set()
  const unexpected = (await readdir(runDir)).filter(entry => !allowed.has(entry))
  if (unexpected.length > 0) throw new Error(`idle soak run-dir is not clean: ${unexpected.join(",")}`)
  return runDir
}

export function validateIdleSoakDetachContract(contract, { source, childPid }) {
  if (contract?.childPid !== childPid) throw new Error("idle soak detach contract child pid mismatch")
  if (contract?.preflight?.status !== "passed" || !sameSource(contract.preflight.source, source)) {
    throw new Error("idle soak detach requires same-source preflight")
  }
  if (contract?.smoke?.status !== "passed" || contract.smoke?.smoke !== true
    || !sameSource(contract.smoke.source, source)) throw new Error("idle soak detach requires same-source smoke")
  return contract
}

function sameSource(actual, expected) {
  return actual?.commit === expected?.commit && actual?.dirty === false && expected?.dirty === false
}

function assertResourceEvidence(sample, label) {
  if (!sample?.owned || !sample?.disk || !sample?.host) throw new Error(`idle soak result lacks ${label} resource evidence`)
  for (const value of [sample.owned.processCount, sample.owned.rssBytes, sample.owned.cpuPercent,
    sample.disk.availableBytes, sample.disk.totalBytes, sample.host.totalMemoryBytes, sample.host.freeMemoryBytes,
    ...(sample.host.loadAverage ?? [])]) {
    if (!Number.isFinite(value) || value < 0) throw new Error(`idle soak result has invalid ${label} resource evidence`)
  }
}

function assertExternal(candidate, repoRoot) {
  const relative = path.relative(path.resolve(repoRoot), candidate)
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error("idle soak evidence must stay outside repositories")
  }
}

function integer(value, label, minimum, maximum) {
  if (!/^[0-9]+$/.test(value ?? "")) throw new Error(`${label} must be an integer from ${minimum} to ${maximum}`)
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${label} must be an integer from ${minimum} to ${maximum}`)
  }
  return parsed
}

function optionalInteger(value, label, minimum, maximum) {
  return value == null ? null : integer(value, label, minimum, maximum)
}

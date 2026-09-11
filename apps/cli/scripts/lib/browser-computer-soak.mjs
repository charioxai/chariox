import path from "node:path"
import { createHash } from "node:crypto"

export const DEFAULT_SOAK_DURATION_SECONDS = 8 * 60 * 60
export const SMOKE_DURATION_SECONDS = 12
export const MAXIMUM_SOAK_DURATION_SECONDS = 24 * 60 * 60
export const GATE_RECEIPT_MAX_AGE_MS = 60 * 60 * 1_000
export const SELKIES_REQUIRED_PROTOCOL_VERSION = 322

export function parseBrowserComputerSoakArgs(argv, { repoRoot, homeDir }) {
  const values = new Map()
  const flags = new Set()
  const valueFlags = new Set([
    "--duration-seconds",
    "--activity-interval-seconds",
    "--sample-interval-seconds",
    "--evidence-root",
    "--run-dir",
    "--display-number",
    "--debug-port",
    "--viewer-port",
    "--viewer-backend",
    "--max-cadence-gap-seconds",
    "--max-rss-mib",
    "--max-cpu-percent",
    "--max-processes",
    "--max-disk-growth-mib",
    "--max-open-files",
    "--max-network-mib",
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
  const durationSeconds = smoke
    ? SMOKE_DURATION_SECONDS
    : integer(values.get("--duration-seconds") ?? String(DEFAULT_SOAK_DURATION_SECONDS), "duration-seconds", 1, MAXIMUM_SOAK_DURATION_SECONDS)
  const activityIntervalSeconds = smoke
    ? 1
    : integer(values.get("--activity-interval-seconds") ?? "10", "activity-interval-seconds", 1, 300)
  const sampleIntervalSeconds = smoke
    ? 1
    : integer(values.get("--sample-interval-seconds") ?? "10", "sample-interval-seconds", 1, 300)
  const evidenceRoot = path.resolve(values.get("--evidence-root")
    ?? path.join(homeDir, ".chariox", "dev", "browser-computer-use-soak"))
  assertExternalEvidencePath(evidenceRoot, repoRoot)
  const runDir = values.has("--run-dir") ? path.resolve(values.get("--run-dir")) : null
  if (runDir) {
    assertExternalEvidencePath(runDir, repoRoot)
    const relative = path.relative(evidenceRoot, runDir)
    if (relative.startsWith(`..${path.sep}`) || relative === ".." || path.isAbsolute(relative)) {
      throw new Error("run-dir must be inside evidence-root")
    }
  }
  const preflight = flags.has("--preflight")
  const detach = flags.has("--detach")
  const internalRun = flags.has("--internal-run")
  if (preflight && detach) throw new Error("--preflight cannot be combined with --detach")
  if (internalRun && (preflight || detach)) throw new Error("--internal-run cannot be combined with --preflight or --detach")

  const viewerBackend = values.get("--viewer-backend") ?? process.env.CHARIOX_SLICE_VIEWER_BACKEND ?? "selkies"
  if (!new Set(["selkies", "novnc"]).has(viewerBackend)) throw new Error("viewer-backend must be selkies or novnc")
  const defaultCadenceGapSeconds = Math.max(30, activityIntervalSeconds * 3, sampleIntervalSeconds * 3)
  const maxCadenceGapSeconds = integer(values.get("--max-cadence-gap-seconds") ?? String(defaultCadenceGapSeconds), "max-cadence-gap-seconds", 2, 900)
  if (maxCadenceGapSeconds <= Math.max(activityIntervalSeconds, sampleIntervalSeconds)) {
    throw new Error("max-cadence-gap-seconds must exceed both activity and sample intervals")
  }

  return {
    mode: preflight ? "preflight" : detach ? "detach" : "run",
    smoke,
    durationSeconds,
    activityIntervalSeconds,
    sampleIntervalSeconds,
    evidenceRoot,
    runDir,
    displayNumber: optionalInteger(values.get("--display-number"), "display-number", 60, 199),
    debugPort: optionalInteger(values.get("--debug-port"), "debug-port", 1024, 65_535),
    viewerPort: optionalInteger(values.get("--viewer-port"), "viewer-port", 1024, 65_535),
    viewerBackend,
    limits: {
      maxCadenceGapMs: maxCadenceGapSeconds * 1_000,
      maxRssBytes: integer(values.get("--max-rss-mib") ?? "4096", "max-rss-mib", 128, 65_536) * 1024 * 1024,
      maxCpuPercent: integer(values.get("--max-cpu-percent") ?? "800", "max-cpu-percent", 1, 10_000),
      maxProcesses: integer(values.get("--max-processes") ?? "96", "max-processes", 1, 10_000),
      maxDiskGrowthBytes: integer(values.get("--max-disk-growth-mib") ?? "1024", "max-disk-growth-mib", 1, 1_048_576) * 1024 * 1024,
      maxOpenFiles: integer(values.get("--max-open-files") ?? "8192", "max-open-files", 1, 1_000_000),
      maxNetworkBytes: integer(values.get("--max-network-mib") ?? "65536", "max-network-mib", 1, 1_048_576) * 1024 * 1024,
    },
    internalRun,
    help: flags.has("--help") || flags.has("-h"),
  }
}

export function buildSoakPaths(evidenceRoot, runId) {
  const runDir = path.join(evidenceRoot, runId)
  return {
    runDir,
    log: path.join(runDir, "runner.log"),
    pid: path.join(runDir, "runner.pid"),
    status: path.join(runDir, "status.json"),
    result: path.join(runDir, "result.json"),
    samples: path.join(runDir, "resource-samples.jsonl"),
    activity: path.join(runDir, "activity.jsonl"),
    cleanup: path.join(runDir, "cleanup-ledger.json"),
    failure: path.join(runDir, "failure.json"),
    preflight: path.join(runDir, "preflight.json"),
  }
}

export function buildGateReceiptPaths(evidenceRoot) {
  return {
    preflight: path.join(evidenceRoot, "latest-preflight-receipt.json"),
    smoke: path.join(evidenceRoot, "latest-smoke-receipt.json"),
  }
}

export function detachedLaunchSummary(paths, pid) {
  return {
    status: "started",
    pid,
    runDir: paths.runDir,
    statusPath: paths.status,
    pidPath: paths.pid,
    log: paths.log,
    result: paths.result,
    samples: paths.samples,
    activity: paths.activity,
    cleanup: paths.cleanup,
    failure: paths.failure,
    preflight: paths.preflight,
  }
}

export function validateCompletedSoakResult(value) {
  if (!value || typeof value !== "object" || value.schema !== "chariox.browser_computer_soak.v1" || value.status !== "passed") {
    throw new Error("soak result must be a passed chariox.browser_computer_soak.v1 report")
  }
  const provenance = value.provenance
  if (provenance?.schema !== "chariox.browser_computer_soak_provenance.v1"
    || !Number.isFinite(Date.parse(provenance.capturedAt))
    || provenance.source?.dirty !== false
    || !/^[0-9a-f]{40}$/.test(provenance.source?.commit ?? "")
    || !/^[0-9a-f]{40}$/.test(provenance.source?.tree ?? "")
    || typeof provenance.image?.identity !== "string" || provenance.image.identity.length === 0
    || stableJson(provenance.limits) !== stableJson(value.resources?.limits)
    || provenance.viewer?.backend !== value.viewer?.backend
    || stableJson(provenance.source) !== stableJson(value.source)
    || !Number.isSafeInteger(provenance.localDaemonProtocolVersion)) {
    throw new Error("soak result must contain exact clean source, image, viewer, protocol, and limit provenance")
  }
  if (value.firstHealth?.state !== "ready" || !Number.isSafeInteger(value.firstHealth?.process_id)) {
    throw new Error("soak result must contain a ready Browser Controller health result")
  }
  if (value.finalHealth?.state !== "ready" || value.finalHealth.process_id !== value.firstHealth.process_id
    || value.controller?.pid !== value.firstHealth.process_id
    || !freshIso(value.finalHealth.checkedAt, value.completedAt, value.timing?.maxCadenceGapMs)) {
    throw new Error("soak result must contain a fresh final health check from the same Browser Controller")
  }
  const positive = [
    value.activity?.iterations,
    value.activity?.controllerRequests,
    value.activity?.chromiumMutations,
    value.activity?.structuredBrowserActions,
    value.activity?.computerScreenshots,
    value.activity?.computerInputs,
    value.activity?.screenshotDigests,
    value.stream?.binaryFrames,
    value.stream?.binaryBytes,
    value.stream?.changingFrameDigests,
    value.resources?.sampleCount,
  ]
  if (positive.some((entry) => !Number.isFinite(entry) || entry <= 0)) {
    throw new Error("soak result must contain browser, controller, changing stream, and resource evidence")
  }
  if (value.activity.screenshotDigests < 2 || !freshIso(value.activity.lastAt, value.completedAt, value.timing?.maxCadenceGapMs)) {
    throw new Error("soak result must contain changing and fresh Browser/Computer activity")
  }
  if (value.stream?.ready !== true
    || !(value.stream.finalBinaryFrames > value.stream.initialBinaryFrames)
    || !(value.stream.finalActivityCounter > value.stream.initialActivityCounter)
    || !(value.stream.finalChangingFrameDigests > value.stream.initialChangingFrameDigests)
    || !freshIso(value.stream.lastBinaryFrameAt, value.completedAt, value.timing?.maxCadenceGapMs)) {
    throw new Error("soak result must contain fresh, increasing display-stream frame and activity counters")
  }
  if (!Number.isFinite(value.timing?.expectedDurationMs) || !Number.isFinite(value.timing?.monotonicElapsedMs)
    || !Number.isFinite(value.timing?.maxCadenceGapMs) || !Number.isFinite(value.timing?.observedMaxCadenceGapMs)
    || !Number.isFinite(value.timing?.wallMonotonicSkewMs)
    || value.timing.monotonicElapsedMs < value.timing.expectedDurationMs
    || value.timing.observedMaxCadenceGapMs > value.timing.maxCadenceGapMs
    || Math.abs(value.timing.wallMonotonicSkewMs) > value.timing.maxCadenceGapMs) {
    throw new Error("soak result timing must prove its monotonic duration without cadence or suspend gaps")
  }
  if (!Number.isFinite(value.resources?.peakOwnedRssBytes) || value.resources.peakOwnedRssBytes < 0
    || !Number.isFinite(value.resources?.peakOwnedCpuPercent) || value.resources.peakOwnedCpuPercent < 0) {
    throw new Error("soak result must contain valid resource peaks")
  }
  const limits = value.resources?.limits
  const resourceMetrics = [
    value.resources?.peakOwnedRssBytes, value.resources?.peakOwnedCpuPercent,
    value.resources?.peakOwnedProcessCount, value.resources?.diskGrowthBytes,
    value.resources?.peakOwnedOpenFiles, value.resources?.networkBytes,
  ]
  const resourceLimits = limits ? [
    limits.maxRssBytes, limits.maxCpuPercent, limits.maxProcesses,
    limits.maxDiskGrowthBytes, limits.maxOpenFiles, limits.maxNetworkBytes,
  ] : []
  if (value.resources?.withinBounds !== true || !limits
    || resourceMetrics.some((entry) => !Number.isFinite(entry) || entry < 0)
    || resourceLimits.some((entry) => !Number.isFinite(entry) || entry <= 0)
    || value.resources.peakOwnedRssBytes > limits.maxRssBytes
    || value.resources.peakOwnedCpuPercent > limits.maxCpuPercent
    || value.resources.peakOwnedProcessCount > limits.maxProcesses
    || value.resources.diskGrowthBytes > limits.maxDiskGrowthBytes
    || value.resources.peakOwnedOpenFiles > limits.maxOpenFiles
    || value.resources.networkBytes > limits.maxNetworkBytes) {
    throw new Error("soak result must contain bounded CPU, RSS, process, disk, open-file, and network evidence")
  }
  if (value.cleanup?.clean !== true || value.cleanup.pidReuseSafe !== true
    || value.cleanup.remainingPids?.length !== 0 || value.cleanup.remainingListeners?.length !== 0
    || value.cleanup.leakScan?.clean !== true || value.cleanup.leakScan.matches?.length !== 0) {
    throw new Error("soak result cleanup must be identity-safe and leak-free")
  }
  if (!new Set(["selkies", "novnc"]).has(value.viewer?.backend)) throw new Error("soak result must record a supported viewer backend")
  const protocol = provenance.localDaemonProtocolVersion
  const selkiesEligible = value.viewer.backend === "selkies" && Number.isSafeInteger(protocol)
    && protocol >= SELKIES_REQUIRED_PROTOCOL_VERSION
  const expectedGateReason = selkiesEligible ? null : value.viewer.backend === "novnc"
    ? "novnc_not_final_gate" : `protocol_${SELKIES_REQUIRED_PROTOCOL_VERSION}_not_integrated`
  if (value.viewer.backend === "novnc" && value.gate?.eligible === true) {
    throw new Error("noVNC evidence cannot close the final gate")
  }
  if (value.gate?.eligible !== selkiesEligible || value.gate?.reason !== expectedGateReason) {
    throw new Error("soak result final-gate eligibility does not match its backend and protocol provenance")
  }
  return value
}

export function gateFingerprint(provenance) {
  return createHash("sha256").update(stableJson({
    source: provenance?.source,
    image: provenance?.image,
    limits: provenance?.limits,
    viewer: provenance?.viewer,
  })).digest("hex")
}

export function validateGatePrerequisites({ preflight, smoke, provenance, now = Date.now() }) {
  if (provenance?.source?.dirty !== false) throw new Error("gate requires a clean source tree")
  const expected = gateFingerprint(provenance)
  for (const [phase, receipt] of [["preflight", preflight], ["smoke", smoke]]) {
    if (receipt?.schema !== "chariox.browser_computer_soak_gate_receipt.v1"
      || receipt.phase !== phase || receipt.status !== "passed" || receipt.fingerprint !== expected) {
      throw new Error(`${phase} receipt must pass for the exact clean source, image, viewer, and limits`)
    }
    const completed = Date.parse(receipt.completedAt)
    if (!Number.isFinite(completed) || completed > now || now - completed > GATE_RECEIPT_MAX_AGE_MS) {
      throw new Error(`${phase} receipt is stale`)
    }
  }
  if (smoke.cleanup?.clean !== true) throw new Error("smoke cleanup must be clean")
  if (Date.parse(smoke.completedAt) < Date.parse(preflight.completedAt)) throw new Error("smoke receipt predates preflight")
}

function assertExternalEvidencePath(candidate, repoRoot) {
  const relative = path.relative(path.resolve(repoRoot), candidate)
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error("browser/computer soak evidence must stay outside repositories")
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

function freshIso(value, completedAt, maximumAgeMs) {
  const time = Date.parse(value)
  const completed = Date.parse(completedAt)
  return Number.isFinite(time) && Number.isFinite(completed) && time <= completed && completed - time <= maximumAgeMs
}

function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`
  return JSON.stringify(value)
}

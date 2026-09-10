import path from "node:path"

export const DEFAULT_SOAK_DURATION_SECONDS = 8 * 60 * 60
export const SMOKE_DURATION_SECONDS = 12
export const MAXIMUM_SOAK_DURATION_SECONDS = 24 * 60 * 60

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
    cleanup: path.join(runDir, "cleanup-ledger.json"),
    failure: path.join(runDir, "failure.json"),
    preflight: path.join(runDir, "preflight.json"),
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
    cleanup: paths.cleanup,
    failure: paths.failure,
    preflight: paths.preflight,
  }
}

export function validateCompletedSoakResult(value) {
  if (!value || typeof value !== "object" || value.schema !== "chariox.browser_computer_soak.v1" || value.status !== "passed") {
    throw new Error("soak result must be a passed chariox.browser_computer_soak.v1 report")
  }
  if (value.firstHealth?.state !== "ready" || !Number.isSafeInteger(value.firstHealth?.process_id)) {
    throw new Error("soak result must contain a ready Browser Controller health result")
  }
  const positive = [
    value.activity?.iterations,
    value.activity?.controllerRequests,
    value.activity?.chromiumMutations,
    value.stream?.binaryFrames,
    value.stream?.binaryBytes,
    value.stream?.changingFrameDigests,
    value.resources?.sampleCount,
  ]
  if (positive.some((entry) => !Number.isFinite(entry) || entry <= 0)) {
    throw new Error("soak result must contain browser, controller, changing stream, and resource evidence")
  }
  if (!Number.isFinite(value.resources?.peakOwnedRssBytes) || value.resources.peakOwnedRssBytes < 0
    || !Number.isFinite(value.resources?.peakOwnedCpuPercent) || value.resources.peakOwnedCpuPercent < 0) {
    throw new Error("soak result must contain valid resource peaks")
  }
  if (value.cleanup?.clean !== true) throw new Error("soak result cleanup must be clean")
  return value
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

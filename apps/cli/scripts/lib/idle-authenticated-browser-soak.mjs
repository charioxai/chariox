import path from "node:path"

export const DEFAULT_IDLE_SOAK_DURATION_SECONDS = 24 * 60 * 60
export const IDLE_SOAK_SMOKE_DURATION_SECONDS = 15

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
    ?? path.join(homeDir, ".chariox", "dev", "idle-authenticated-browser-soak"))
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
  }
}

export function assertCheckpointAdvanced(previous, current) {
  for (const key of ["healthChecks", "controllerRequests", "authenticatedSessionChecks", "profileMarkerChecks"]) {
    if (!Number.isSafeInteger(current?.[key]) || current[key] <= (previous?.[key] ?? -1)) {
      throw new Error(`${key} did not advance monotonically`)
    }
  }
  return current
}

export function assertResourceCeilings(sample, limits) {
  if (sample.owned.processCount > limits.maxProcesses) throw new Error("owned process ceiling exceeded")
  if (sample.owned.rssBytes > limits.maxRssMb * 1024 ** 2) throw new Error("owned memory ceiling exceeded")
  if (sample.owned.cpuPercent > limits.maxCpuPercent) throw new Error("owned CPU ceiling exceeded")
  if (sample.disk.availableBytes < limits.minFreeDiskMb * 1024 ** 2) throw new Error("free disk floor violated")
  return sample
}

export function assertRetainedEvidenceRedacted(texts, forbiddenValues) {
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
  for (const key of ["healthChecks", "controllerRequests", "authenticatedSessionChecks", "profileMarkerChecks"]) {
    if (!Number.isSafeInteger(value.counters?.[key]) || value.counters[key] <= 0) throw new Error("idle soak result lacks health evidence")
  }
  if (!Number.isSafeInteger(value.resources?.sampleCount) || value.resources.sampleCount <= 0
    || value.resources.ceilingsRespected !== true) throw new Error("idle soak result lacks ceiling evidence")
  if (value.redaction?.passed !== true) throw new Error("idle soak result lacks redaction evidence")
  if (value.cleanup?.clean !== true) throw new Error("idle soak result cleanup is not clean")
  return value
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

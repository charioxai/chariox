import { access, statfs } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

const SLICE_CONTAINER_PREFIX = "chariox-slice-"
const CONTROL_CHARACTERS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g
const SENSITIVE_KEY = /(?:authorization|cookie|credential|password|passphrase|secret|token|api[_-]?key|private[_-]?key|client[_-]?secret|access[_-]?key)/i
const SECRET_TEXT_PATTERNS = [
  /\b(?:Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi,
  /(\b(?:token|password|secret|api[_-]?key|authorization|cookie)\b\s*[:=]\s*)[^\s,;]+/gi,
]

export const BROWSER_COMPUTER_GUARD_SCHEMA = "chariox.browser_computer_m0_guard.v1"
export const BROWSER_COMPUTER_SAMPLE_PHASES = Object.freeze(["before", "during", "after"])
export const BROWSER_COMPUTER_FAULT_CHECKPOINTS = Object.freeze([
  "before-operation",
  "before-docker-save",
  "after-docker-save",
  "before-docker-remove",
  "after-docker-remove",
  "before-docker-restore",
  "after-docker-restore",
  "before-browser-start",
  "during-browser",
  "after-browser-stop",
  "after-operation",
  "before-cleanup",
  "after-cleanup",
])
export const BROWSER_COMPUTER_DOCKER_ACTIONS = Object.freeze(["save", "remove", "restore"])

const CAP_ALIASES = Object.freeze({
  diskBytes: Object.freeze(["diskBytes", "maxDiskBytes", "maxDiskGrowthBytes"]),
  memoryBytes: Object.freeze(["memoryBytes", "maxMemoryBytes", "maxRssBytes"]),
  processCount: Object.freeze(["processCount", "maxProcessCount", "maxProcesses"]),
  logBytes: Object.freeze(["logBytes", "maxLogBytes", "maxLogGrowthBytes"]),
})

export function parseBrowserComputerByteBudget(value) {
  if (value === undefined || value === null) return undefined
  if (typeof value === "string" && value.trim() === "") return undefined
  return Number(value)
}

export function normalizeBrowserComputerCaps(value, { required = true } = {}) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("browser/computer resource caps are required")
  }
  const caps = {}
  for (const [name, aliases] of Object.entries(CAP_ALIASES)) {
    const alias = aliases.find((candidate) => Object.prototype.hasOwnProperty.call(value, candidate)
      && value[candidate] !== undefined && value[candidate] !== null)
    if (!alias) {
      if (required) throw new Error(`browser/computer resource cap ${name} is required`)
      continue
    }
    const candidate = Number(value[alias])
    if (!Number.isSafeInteger(candidate) || candidate < 0) {
      throw new Error(`browser/computer resource cap ${name} must be a non-negative safe integer`)
    }
    caps[name] = candidate
  }
  return Object.freeze(caps)
}

export const normalizeBrowserComputerResourceCaps = normalizeBrowserComputerCaps

export function defaultBrowserComputerEvidenceDir(runId, homeDir = os.homedir()) {
  if (!nonEmptyString(runId)) throw new Error("browser/computer drill run id is required")
  return path.join(homeDir, ".codex", "evidence", "browser-computer-use", "m0", runId)
}

export function assertBrowserComputerEvidencePath(evidenceDir, repoRoots) {
  if (!nonEmptyString(evidenceDir)) throw new Error("browser/computer drill evidence directory is required")
  const resolvedEvidenceDir = path.resolve(evidenceDir)
  for (const repoRoot of names(repoRoots)) {
    const relative = path.relative(path.resolve(repoRoot), resolvedEvidenceDir)
    if (relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative))) {
      throw new Error(`browser/computer drill evidence must stay outside repositories: ${resolvedEvidenceDir}`)
    }
  }
  return resolvedEvidenceDir
}

export async function collectBrowserComputerResourceSnapshot({
  runCommand,
  filesystemPath,
  platform = process.platform,
  phase = null,
  sampleId = phase,
  now = () => new Date(),
  processCount,
  logBytes,
}) {
  if (typeof runCommand !== "function") throw new Error("runCommand is required")
  if (!nonEmptyString(filesystemPath)) throw new Error("filesystemPath is required")

  const [disk, dockerContainers, dockerVolumes, dockerImages, availableMemoryBytes, resolvedProcessCount, resolvedLogBytes] = await Promise.all([
    statfs(filesystemPath),
    dockerNames(runCommand, ["ps", "-a", "--format", "{{.Names}}"]),
    dockerNames(runCommand, ["volume", "ls", "--format", "{{.Name}}"]),
    dockerImageRefs(runCommand),
    resolveAvailableMemoryBytes(runCommand, platform),
    resolveMetric(processCount),
    resolveMetric(logBytes),
  ])
  const blockSize = Number(disk.bsize)
  const capturedAt = toIso(now)
  const snapshot = {
    schema: BROWSER_COMPUTER_GUARD_SCHEMA,
    capturedAt,
    phase: phase === null ? null : String(phase),
    sampleId: sampleId === null || sampleId === undefined ? null : String(sampleId),
    platform,
    memory: {
      totalBytes: os.totalmem(),
      availableBytes: Math.min(os.totalmem(), availableMemoryBytes),
    },
    disk: {
      totalBytes: Number(disk.blocks) * blockSize,
      availableBytes: Number(disk.bavail) * blockSize,
      filesystemPath: path.resolve(filesystemPath),
    },
    docker: {
      containers: dockerContainers,
      volumes: dockerVolumes,
      images: dockerImages,
    },
  }
  if (resolvedProcessCount !== undefined) snapshot.process = { count: resolvedProcessCount }
  if (resolvedLogBytes !== undefined) snapshot.logs = { bytes: resolvedLogBytes }
  return snapshot
}

export function evaluateBrowserComputerPreflight(snapshot, options = {}) {
  // Budgets describe the next operation's estimated peak plus its recovery
  // reserve. Free percentages are evidence, never a global execution gate.
  const requiredMemoryBytes = options.requiredMemoryBytes ?? 0
  const requiredDiskBytes = options.requiredDiskBytes ?? 0
  const allowExistingHeadedSlices = options.allowExistingHeadedSlices === true
  assertByteBudget(requiredMemoryBytes, "requiredMemoryBytes")
  assertByteBudget(requiredDiskBytes, "requiredDiskBytes")

  const memoryHeadroom = ratio(snapshot?.memory?.availableBytes, snapshot?.memory?.totalBytes)
  const diskHeadroom = ratio(snapshot?.disk?.availableBytes, snapshot?.disk?.totalBytes)
  const existingSliceContainers = names(snapshot?.docker?.containers)
    .filter((name) => name.startsWith(SLICE_CONTAINER_PREFIX))
  const violations = []
  const warnings = []
  const availableMemoryBytes = numeric(snapshot?.memory?.availableBytes)
  const availableDiskBytes = numeric(snapshot?.disk?.availableBytes)

  if (availableMemoryBytes <= 0 || availableMemoryBytes < requiredMemoryBytes) {
    violations.push(`available memory ${availableMemoryBytes} bytes cannot cover the operation budget ${requiredMemoryBytes} bytes`)
  }
  if (availableDiskBytes <= 0 || availableDiskBytes < requiredDiskBytes) {
    violations.push(`available disk ${availableDiskBytes} bytes cannot cover the operation budget ${requiredDiskBytes} bytes`)
  }
  if (options.requiredMemoryBytes === undefined || options.requiredDiskBytes === undefined) {
    warnings.push("operation budget not fully specified; resource percentages are observational, not proof that the next operation fits")
  }
  if (!allowExistingHeadedSlices && existingSliceContainers.length > 0) {
    violations.push(`existing slice containers make a single-slice developer run unsafe: ${existingSliceContainers.join(", ")}`)
  }

  return {
    ok: violations.length === 0,
    memoryHeadroom,
    diskHeadroom,
    requiredMemoryBytes,
    requiredDiskBytes,
    existingSliceContainers,
    violations,
    warnings,
  }
}

export function assertBrowserComputerPreflight(snapshot, options = {}) {
  const result = evaluateBrowserComputerPreflight(snapshot, options)
  if (!result.ok) {
    throw new Error(`browser/computer drill resource preflight failed:\n- ${result.violations.join("\n- ")}`)
  }
  return result
}

/**
 * Validate exactly one deterministic before/during/after resource sample set.
 * Disk and log caps are operation growth caps; memory and process caps are
 * peak caps. Missing process or log measurements fail closed when caps apply.
 */
export function evaluateBrowserComputerResourceCaps(samples, caps) {
  const normalizedCaps = normalizeBrowserComputerCaps(caps)
  const violations = []
  const rows = []
  if (!Array.isArray(samples) || samples.length !== BROWSER_COMPUTER_SAMPLE_PHASES.length) {
    violations.push("resource evidence must contain exactly one before, during, and after sample")
  }

  for (let index = 0; index < BROWSER_COMPUTER_SAMPLE_PHASES.length; index += 1) {
    const phase = BROWSER_COMPUTER_SAMPLE_PHASES[index]
    const sample = Array.isArray(samples) ? samples[index] : undefined
    if (!sample || sample.phase !== phase) {
      violations.push(`resource evidence is missing the ${phase} sample at deterministic position ${index}`)
      continue
    }
    const row = resourceMetrics(sample, phase, violations)
    if (row) rows.push({ phase, ...row })
  }

  const metrics = {
    diskGrowthBytes: null,
    peakMemoryBytes: null,
    peakProcessCount: null,
    logGrowthBytes: null,
    peakLogBytes: null,
  }
  if (rows.length === BROWSER_COMPUTER_SAMPLE_PHASES.length) {
    const baseline = rows[0]
    metrics.diskGrowthBytes = Math.max(...rows.map((row) => Math.max(0, row.diskUsedBytes - baseline.diskUsedBytes)))
    metrics.peakMemoryBytes = Math.max(...rows.map((row) => row.memoryUsedBytes))
    metrics.peakProcessCount = Math.max(...rows.map((row) => row.processCount))
    metrics.logGrowthBytes = Math.max(...rows.map((row) => Math.max(0, row.logBytes - baseline.logBytes)))
    metrics.peakLogBytes = Math.max(...rows.map((row) => row.logBytes))
    if (metrics.diskGrowthBytes > normalizedCaps.diskBytes) {
      violations.push(`disk growth ${metrics.diskGrowthBytes} bytes exceeds cap ${normalizedCaps.diskBytes} bytes`)
    }
    if (metrics.peakMemoryBytes > normalizedCaps.memoryBytes) {
      violations.push(`peak memory ${metrics.peakMemoryBytes} bytes exceeds cap ${normalizedCaps.memoryBytes} bytes`)
    }
    if (metrics.peakProcessCount > normalizedCaps.processCount) {
      violations.push(`peak process count ${metrics.peakProcessCount} exceeds cap ${normalizedCaps.processCount}`)
    }
    if (metrics.logGrowthBytes > normalizedCaps.logBytes) {
      violations.push(`log growth ${metrics.logGrowthBytes} bytes exceeds cap ${normalizedCaps.logBytes} bytes`)
    }
  }

  return {
    schema: BROWSER_COMPUTER_GUARD_SCHEMA,
    ok: violations.length === 0,
    phases: BROWSER_COMPUTER_SAMPLE_PHASES,
    caps: normalizedCaps,
    metrics,
    violations,
  }
}

export const evaluateBrowserComputerResourceSamples = evaluateBrowserComputerResourceCaps

export function assertBrowserComputerResourceCaps(samples, caps) {
  const result = evaluateBrowserComputerResourceCaps(samples, caps)
  if (!result.ok) {
    throw new Error(`browser/computer resource caps failed:\n- ${result.violations.join("\n- ")}`)
  }
  return result
}

export const assertBrowserComputerResourceSamples = assertBrowserComputerResourceCaps

export async function evaluateBrowserComputerCleanup({
  before,
  after,
  ownedContainers = [],
  ownedVolumes = [],
  tempRoots = [],
  childProcesses = [],
  cleanupActions = [],
  cleanupCommands = [],
  allowRetainedResources = false,
}) {
  const beforeInventory = dockerInventory(before)
  const afterInventory = dockerInventory(after)
  const afterContainers = new Set(afterInventory.containers)
  const afterVolumes = new Set(afterInventory.volumes)
  const beforeContainers = new Set(beforeInventory.containers)
  const beforeVolumes = new Set(beforeInventory.volumes)
  const ownedContainerNames = new Set(names(ownedContainers))
  const ownedVolumeNames = new Set(names(ownedVolumes))
  const violations = [...beforeInventory.violations, ...afterInventory.violations]

  const createdContainers = difference(afterContainers, beforeContainers)
  const createdVolumes = difference(afterVolumes, beforeVolumes)
  const removedOwnedContainers = [...ownedContainerNames].filter((name) => beforeContainers.has(name) && !afterContainers.has(name))
  const removedOwnedVolumes = [...ownedVolumeNames].filter((name) => beforeVolumes.has(name) && !afterVolumes.has(name))
  const removedUnownedContainers = [...beforeContainers].filter((name) => !afterContainers.has(name) && !ownedContainerNames.has(name))
  const removedUnownedVolumes = [...beforeVolumes].filter((name) => !afterVolumes.has(name) && !ownedVolumeNames.has(name))
  const remainingOwnedContainers = [...ownedContainerNames].filter((name) => afterContainers.has(name))
  const remainingOwnedVolumes = [...ownedVolumeNames].filter((name) => afterVolumes.has(name))

  for (const name of removedUnownedContainers) violations.push(`pre-existing unowned container disappeared: ${name}`)
  for (const name of removedUnownedVolumes) violations.push(`pre-existing unowned volume disappeared: ${name}`)

  if (!allowRetainedResources) {
    for (const name of remainingOwnedContainers) violations.push(`owned container remains: ${name}`)
    for (const name of remainingOwnedVolumes) violations.push(`owned volume remains: ${name}`)
    for (const name of createdContainers) {
      if (name.startsWith(SLICE_CONTAINER_PREFIX) && !ownedContainerNames.has(name)) {
        violations.push(`new slice container remains: ${name}`)
      }
    }
    for (const name of createdVolumes) {
      if (name.startsWith(SLICE_CONTAINER_PREFIX) && !ownedVolumeNames.has(name)) {
        violations.push(`new slice volume remains: ${name}`)
      }
    }
    for (const tempRoot of tempRoots) {
      if (await exists(tempRoot)) violations.push(`temporary root remains: ${tempRoot}`)
    }
  }

  const actionAccounting = accountCleanupActions(cleanupActions, ownedContainerNames, ownedVolumeNames, violations)
  for (const command of cleanupCommands) {
    const result = evaluateBrowserComputerDockerPreconditions(command)
    if (!result.ok) violations.push(...result.violations.map((entry) => `cleanup command rejected: ${entry}`))
  }
  for (const child of childProcesses) {
    if (child?.exitCode === null && child?.signalCode === null) {
      violations.push(`child process remains alive: ${child.drillLabel ?? child.spawnfile ?? "unknown"}`)
    }
  }

  return {
    schema: BROWSER_COMPUTER_GUARD_SCHEMA,
    ok: violations.length === 0,
    violations,
    memoryAvailableDeltaBytes: numeric(after?.memory?.availableBytes) - numeric(before?.memory?.availableBytes),
    diskAvailableDeltaBytes: numeric(after?.disk?.availableBytes) - numeric(before?.disk?.availableBytes),
    cleanupAccounting: {
      created: { containers: [...createdContainers], volumes: [...createdVolumes] },
      removedOwned: { containers: removedOwnedContainers, volumes: removedOwnedVolumes },
      removedUnowned: { containers: removedUnownedContainers, volumes: removedUnownedVolumes },
      remainingOwned: { containers: remainingOwnedContainers, volumes: remainingOwnedVolumes },
      actions: actionAccounting,
    },
  }
}

export function assertBrowserComputerCleanup(result) {
  if (!result?.ok) {
    throw new Error(`browser/computer drill cleanup failed:\n- ${(result?.violations ?? ["unknown cleanup failure"]).join("\n- ")}`)
  }
  return result
}

export function evaluateBrowserComputerDockerPreconditions({
  action,
  before,
  ownedContainers = [],
  ownedVolumes = [],
  targetContainers = [],
  targetVolumes = [],
  imageRef,
  savePath,
  restorePath,
  saved = false,
  removed = false,
  saveReceipt,
  removeReceipt,
  command,
} = {}) {
  const violations = []
  if (!BROWSER_COMPUTER_DOCKER_ACTIONS.includes(action)) {
    violations.push(`Docker mutation action must be one of ${BROWSER_COMPUTER_DOCKER_ACTIONS.join(", ")}`)
  }
  const inventory = dockerInventory(before)
  violations.push(...inventory.violations)
  const owned = {
    containers: new Set(names(ownedContainers)),
    volumes: new Set(names(ownedVolumes)),
  }
  const targets = {
    containers: names(targetContainers),
    volumes: names(targetVolumes),
  }
  const allTargets = [...targets.containers, ...targets.volumes]
  if (allTargets.some(isBroadResourceSelector)) {
    violations.push("Docker mutation must name exact owned resources; broad prune selectors are forbidden")
  }
  if (command !== undefined) {
    validateDockerCommand(command, {
      action,
      targetContainers: targets.containers,
      targetVolumes: targets.volumes,
      imageRef,
    }, violations)
  }

  const saveReady = saved === true || saveReceipt?.ok === true
  const removeReady = removed === true || removeReceipt?.ok === true
  if (action === "save") {
    requireSafeToken(imageRef, "Docker save image", violations)
    requireSafeToken(savePath, "Docker save evidence path", violations)
    if (!Array.isArray(inventory.images)) {
      violations.push("Docker save requires an authoritative image inventory")
    } else if (nonEmptyString(imageRef) && !inventory.images.includes(imageRef)) {
      violations.push(`Docker save image is absent from the authoritative inventory: ${imageRef}`)
    }
  }
  if (action === "remove") {
    if (!saveReady) violations.push("Docker remove requires a successful exact save receipt")
    if (allTargets.length === 0) violations.push("Docker remove requires at least one exact owned resource")
    for (const name of targets.containers) {
      if (!owned.containers.has(name)) violations.push(`Docker remove target is not an owned container: ${name}`)
      if (!inventory.containers.includes(name)) violations.push(`Docker remove container is absent from the preflight inventory: ${name}`)
    }
    for (const name of targets.volumes) {
      if (!owned.volumes.has(name)) violations.push(`Docker remove target is not an owned volume: ${name}`)
      if (!inventory.volumes.includes(name)) violations.push(`Docker remove volume is absent from the preflight inventory: ${name}`)
    }
  }
  if (action === "restore") {
    if (!saveReady) violations.push("Docker restore requires a successful exact save receipt")
    if (!removeReady) violations.push("Docker restore requires a successful exact remove receipt")
    requireSafeToken(restorePath, "Docker restore evidence path", violations)
    if (allTargets.length === 0 && !nonEmptyString(imageRef)) {
      violations.push("Docker restore requires an exact resource or image target")
    }
    for (const name of targets.containers) {
      if (!owned.containers.has(name)) violations.push(`Docker restore target is not an owned container: ${name}`)
    }
    for (const name of targets.volumes) {
      if (!owned.volumes.has(name)) violations.push(`Docker restore target is not an owned volume: ${name}`)
    }
  }

  return {
    schema: BROWSER_COMPUTER_GUARD_SCHEMA,
    action,
    ok: violations.length === 0,
    targets,
    violations,
  }
}

export const validateBrowserComputerDockerMutation = evaluateBrowserComputerDockerPreconditions
export const evaluateBrowserComputerDockerMutation = evaluateBrowserComputerDockerPreconditions

export function assertBrowserComputerDockerPreconditions(input) {
  const result = evaluateBrowserComputerDockerPreconditions(input)
  if (!result.ok) {
    throw new Error(`browser/computer Docker ${input?.action ?? "mutation"} preconditions failed:\n- ${result.violations.join("\n- ")}`)
  }
  return result
}

export const assertBrowserComputerDockerMutationPreconditions = assertBrowserComputerDockerPreconditions

export function parseBrowserComputerDockerMutationArgv(command, action) {
  const args = commandArgs(command)
  const violations = []
  if (!args || args.length < 2 || !["docker", "podman"].includes(path.basename(args[0]))) {
    violations.push("Docker mutation command must be an explicit docker/podman argv")
    return { args: null, action, subcommand: null, affected: emptyDockerResources(), violations }
  }

  const subcommand = args[1]
  const nestedSubcommand = subcommand === "volume" ? args[2] : null
  const start = nestedSubcommand ? 3 : 2
  const optionValues = nestedSubcommand === "rm" || subcommand === "rm"
    ? new Set(["--filter"])
    : subcommand === "save"
      ? new Set(["-o", "--output", "--platform"])
      : subcommand === "load"
        ? new Set(["-i", "--input", "--platform"])
        : subcommand === "create"
          ? new Set(["--name", "--label", "--env", "-e", "--network", "--volume", "-v", "--mount"])
          : new Set()
  const positionals = dockerPositionalArgs(args, start, optionValues)
  const affected = emptyDockerResources()
  if (action === "save" && subcommand === "save") {
    affected.images = positionals
  } else if (action === "remove" && subcommand === "rm") {
    affected.containers = positionals
  } else if (action === "remove" && subcommand === "volume" && nestedSubcommand === "rm") {
    affected.volumes = positionals
  } else if (action === "restore" && subcommand === "create") {
    affected.images = positionals.slice(0, 1)
  } else if (positionals.length > 0 && action !== "restore") {
    violations.push(`Docker ${action ?? "mutation"} argv has unrecognized positional resources: ${positionals.join(", ")}`)
  }

  return { args, action, subcommand, nestedSubcommand, affected, violations }
}

export const parseBrowserComputerDockerArgv = parseBrowserComputerDockerMutationArgv

export async function runBrowserComputerFaultGuard({
  operation = async () => undefined,
  cleanup = async () => ({ clean: true }),
  faultAt = null,
  now = () => new Date(),
  onCheckpoint,
  secretValues = [],
} = {}) {
  if (typeof operation !== "function") throw new Error("browser/computer guard operation is required")
  if (typeof cleanup !== "function") throw new Error("browser/computer guard cleanup is required")
  if (faultAt !== null && faultAt !== undefined && !BROWSER_COMPUTER_FAULT_CHECKPOINTS.includes(faultAt)) {
    throw new Error(`unknown browser/computer fault checkpoint: ${faultAt}`)
  }
  const checkpoints = []
  let operationError = null
  let cleanupError = null
  let cleanupResult
  let requestedFaultHits = 0
  const checkpoint = async (name, details = {}) => {
    const checkpointIndex = BROWSER_COMPUTER_FAULT_CHECKPOINTS.indexOf(name)
    if (checkpointIndex < 0) {
      throw new Error(`unknown browser/computer fault checkpoint: ${name}`)
    }
    const previous = checkpoints.at(-1)
    if (checkpoints.some((entry) => entry.name === name)) {
      throw new Error(`browser/computer fault checkpoint must be reached exactly once: ${name}`)
    }
    if (previous && checkpointIndex <= BROWSER_COMPUTER_FAULT_CHECKPOINTS.indexOf(previous.name)) {
      throw new Error(`browser/computer fault checkpoints are out of order: ${previous.name} -> ${name}`)
    }
    const entry = redactBrowserComputerEvidence({
      name,
      ordinal: checkpoints.length,
      capturedAt: toIso(now),
      details,
    }, { secretValues })
    checkpoints.push(entry)
    if (typeof onCheckpoint === "function") await onCheckpoint(entry)
    if (faultAt === name) {
      requestedFaultHits += 1
      throw new Error(`injected browser/computer fault at ${name}`)
    }
    return entry
  }

  try {
    await checkpoint("before-operation")
    await operation({ checkpoint })
    await checkpoint("after-operation")
  } catch (error) {
    operationError = error
  }

  try {
    try {
      await checkpoint("before-cleanup")
    } catch (error) {
      operationError ??= error
    }
    try {
      cleanupResult = await cleanup({ interrupted: operationError !== null, error: operationError, checkpoint })
      cleanupError = cleanupResultFailure(cleanupResult)
    } catch (error) {
      cleanupError = error
    }
    try {
      await checkpoint("after-cleanup")
    } catch (error) {
      cleanupError ??= error
    }
  } catch (error) {
    cleanupError ??= error
  }

  const faultCheckpointError = faultAt !== null && requestedFaultHits !== 1
    ? new Error(`requested browser/computer fault checkpoint was not reached exactly once: ${faultAt}`)
    : null
  const failure = cleanupError ?? operationError ?? faultCheckpointError
  return redactBrowserComputerEvidence({
    schema: BROWSER_COMPUTER_GUARD_SCHEMA,
    status: failure ? "failed" : "passed",
    faultAt: faultAt ?? null,
    faultCheckpoint: {
      requested: faultAt ?? null,
      reached: faultAt === null ? null : requestedFaultHits,
      exercised: faultAt === null ? null : requestedFaultHits === 1,
    },
    interrupted: operationError !== null,
    checkpoints,
    cleanup: cleanupResult ?? null,
    failure: failure ? { message: failure?.message ?? String(failure) } : null,
  }, { secretValues })
}

function cleanupResultFailure(result) {
  if (!result || typeof result !== "object") return null
  if (result.clean === false) {
    return new Error(`cleanup reported clean=false${formatCleanupViolations(result.violations)}`)
  }
  if (result.ok === false) {
    return new Error(`cleanup reported ok=false${formatCleanupViolations(result.violations)}`)
  }
  return null
}

function formatCleanupViolations(violations) {
  return Array.isArray(violations) && violations.length > 0
    ? `: ${violations.map((entry) => String(entry)).join("; ")}`
    : ""
}

export function redactBrowserComputerEvidence(value, { secretValues = [], maxStringLength = 4_000 } = {}) {
  const retainedSecrets = [...new Set(secretValues
    .filter((entry) => typeof entry === "string" && entry.length >= 4))]
    .sort((left, right) => right.length - left.length)
  return redactValue(value, retainedSecrets, maxStringLength)
}

export function serializeBrowserComputerEvidence(value, options = {}) {
  return JSON.stringify(redactBrowserComputerEvidence(value, options))
}

export function assertSecretSafeBrowserComputerEvidence(value, options = {}) {
  const sanitized = redactBrowserComputerEvidence(value, options)
  const serialized = JSON.stringify(sanitized)
  for (const secret of options.secretValues ?? []) {
    if (typeof secret === "string" && secret.length >= 4 && serialized.includes(secret)) {
      throw new Error("browser/computer evidence contains an unredacted secret")
    }
  }
  return sanitized
}

async function resolveAvailableMemoryBytes(runCommand, platform) {
  if (platform === "linux") {
    const result = await runCommand("sh", ["-c", "awk '/^MemAvailable:/ { print $2 * 1024 }' /proc/meminfo"], { timeoutMs: 5_000 })
    const value = Number(result.stdout.trim())
    if (result.code === 0 && Number.isFinite(value) && value >= 0) return value
  }
  if (platform === "darwin") {
    const result = await runCommand("vm_stat", [], { timeoutMs: 5_000 })
    const value = parseDarwinAvailableMemory(result.stdout)
    if (result.code === 0 && value !== null) return value
  }
  return os.freemem()
}

function parseDarwinAvailableMemory(output) {
  const pageSize = Number(String(output).match(/page size of (\d+) bytes/i)?.[1])
  if (!Number.isFinite(pageSize) || pageSize <= 0) return null
  const pages = new Map()
  for (const match of String(output).matchAll(/^Pages ([^:]+):\s+([0-9.]+)\.?$/gm)) {
    pages.set(match[1].trim().toLowerCase(), Number(match[2]))
  }
  const availablePageNames = ["free", "inactive", "speculative", "purgeable"]
  const availablePages = availablePageNames.reduce((total, name) => total + (pages.get(name) ?? 0), 0)
  return availablePages > 0 ? availablePages * pageSize : null
}

async function dockerNames(runCommand, args) {
  const result = await runCommand("docker", args, { timeoutMs: 10_000 })
  if (result.code !== 0) {
    throw new Error(`docker ${args.join(" ")} failed during resource inventory\n${result.stdout}${result.stderr}`)
  }
  return names(result.stdout.split("\n"))
}

async function dockerImageRefs(runCommand) {
  const refs = await dockerNames(runCommand, ["image", "ls", "--no-trunc", "--format", "{{.Repository}}:{{.Tag}}"])
  return refs.filter((ref) => ref !== "<none>:<none>" && !ref.startsWith("<none>:") && !ref.endsWith(":<none>"))
}

function dockerPositionalArgs(args, start, optionValues) {
  const positionals = []
  for (let index = start; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--") {
      positionals.push(...args.slice(index + 1))
      break
    }
    if (!arg.startsWith("-")) {
      positionals.push(arg)
      continue
    }
    if (!arg.includes("=") && optionValues.has(arg)) index += 1
  }
  return positionals
}

function emptyDockerResources() {
  return { containers: [], volumes: [], images: [] }
}

function sameDockerResourceSet(left, right) {
  return ["containers", "volumes", "images"].every((kind) => sameNames(left?.[kind], right?.[kind]))
}

function sameNames(left, right) {
  const a = names(left).sort()
  const b = names(right).sort()
  return a.length === b.length && a.every((value, index) => value === b[index])
}

function formatDockerResources(resources) {
  return JSON.stringify({
    containers: names(resources?.containers),
    volumes: names(resources?.volumes),
    images: names(resources?.images),
  })
}

function resourceMetrics(sample, phase, violations) {
  const disk = sample?.disk
  const memory = sample?.memory
  const processCount = sample?.process?.count ?? sample?.processCount ?? sample?.resources?.processCount
  const logBytes = sample?.logs?.bytes ?? sample?.logBytes ?? sample?.resources?.logBytes
  const diskTotalBytes = safeCounter(disk?.totalBytes)
  const diskAvailableBytes = safeCounter(disk?.availableBytes)
  const memoryTotalBytes = safeCounter(memory?.totalBytes)
  const memoryAvailableBytes = safeCounter(memory?.availableBytes)
  const processValue = safeCounter(processCount)
  const logValue = safeCounter(logBytes)
  if (diskTotalBytes === null || diskTotalBytes <= 0 || diskAvailableBytes === null || diskAvailableBytes > diskTotalBytes) {
    violations.push(`${phase} sample has missing or invalid disk totals/availability`)
  }
  if (memoryTotalBytes === null || memoryTotalBytes <= 0 || memoryAvailableBytes === null || memoryAvailableBytes > memoryTotalBytes) {
    violations.push(`${phase} sample has missing or invalid memory totals/availability`)
  }
  if (processValue === null) violations.push(`${phase} sample is missing a safe process count`)
  if (logValue === null) violations.push(`${phase} sample is missing a safe log byte count`)
  if (diskTotalBytes === null || diskAvailableBytes === null || memoryTotalBytes === null
    || memoryAvailableBytes === null || processValue === null || logValue === null) return null
  const explicitDiskUsed = safeCounter(disk?.usedBytes)
  const explicitMemoryUsed = safeCounter(memory?.usedBytes)
  if (explicitDiskUsed !== null && explicitDiskUsed < 0) violations.push(`${phase} sample has invalid disk usage`)
  if (explicitMemoryUsed !== null && explicitMemoryUsed < 0) violations.push(`${phase} sample has invalid memory usage`)
  return {
    diskUsedBytes: explicitDiskUsed ?? (diskTotalBytes - diskAvailableBytes),
    memoryUsedBytes: explicitMemoryUsed ?? (memoryTotalBytes - memoryAvailableBytes),
    processCount: processValue,
    logBytes: logValue,
  }
}

function accountCleanupActions(actions, ownedContainers, ownedVolumes, violations) {
  const counts = new Map()
  const records = []
  if (!Array.isArray(actions)) {
    violations.push("cleanup action ledger must be an array")
    return records
  }
  for (const action of actions) {
    const kind = action?.kind ?? action?.type
    const name = typeof action?.name === "string" ? action.name : ""
    const owned = kind === "container" ? ownedContainers.has(name) : kind === "volume" ? ownedVolumes.has(name) : false
    if (!owned) violations.push(`cleanup action is not for an owned resource: ${kind ?? "unknown"}/${name || "unknown"}`)
    if (action?.ok === false || action?.status === "failed") violations.push(`cleanup action failed: ${kind ?? "unknown"}/${name || "unknown"}`)
    const key = `${kind ?? "unknown"}:${name}`
    const count = (counts.get(key) ?? 0) + 1
    counts.set(key, count)
    if (count > 1) violations.push(`owned cleanup action repeated: ${key}`)
    records.push({ kind: kind ?? null, name: name || null, ok: action?.ok !== false && action?.status !== "failed" })
  }
  return records
}

function validateDockerCommand(command, { action, targetContainers, targetVolumes, imageRef }, violations) {
  const parsed = parseBrowserComputerDockerMutationArgv(command, action)
  violations.push(...parsed.violations)
  const args = parsed.args
  if (!args) return
  const lower = args.map((entry) => entry.toLowerCase())
  if (lower.some((entry) => entry === "prune" || entry === "system" || entry === "--all" || entry === "*" || /^-[^-]*a/.test(entry))) {
    violations.push("Docker mutation command uses a broad prune or all-resources selector")
  }
  if (action === "save" && args[1] !== "save") violations.push("Docker save precondition requires docker save")
  if (action === "remove" && !(args[1] === "rm" || (args[1] === "volume" && args[2] === "rm"))) {
    violations.push("Docker remove precondition requires an exact rm command")
  }
  if (action === "restore" && !(args[1] === "load" || args[1] === "create")) {
    violations.push("Docker restore precondition requires docker load or exact create")
  }
  if (nonEmptyString(imageRef) && action === "save" && !args.includes(imageRef)) {
    violations.push(`Docker save argv omits its exact image: ${imageRef}`)
  }

  const expected = {
    containers: action === "remove" ? targetContainers : [],
    volumes: action === "remove" ? targetVolumes : [],
    images: action === "save" || (action === "restore" && parsed.subcommand === "create")
      ? (nonEmptyString(imageRef) ? [imageRef] : [])
      : [],
  }
  if (!sameDockerResourceSet(parsed.affected, expected)) {
    violations.push(
      `Docker ${action ?? "mutation"} argv affected resources do not equal declared targets: `
      + `${formatDockerResources(parsed.affected)} != ${formatDockerResources(expected)}`,
    )
  }
}

function dockerInventory(snapshot) {
  const docker = snapshot?.docker
  const violations = []
  if (!Array.isArray(docker?.containers)) violations.push("resource inventory is missing Docker containers")
  if (!Array.isArray(docker?.volumes)) violations.push("resource inventory is missing Docker volumes")
  return {
    containers: names(docker?.containers),
    volumes: names(docker?.volumes),
    images: Array.isArray(docker?.images) ? names(docker.images) : undefined,
    violations,
  }
}

function redactValue(value, retainedSecrets, maxStringLength, key = "") {
  if (SENSITIVE_KEY.test(key)) return "<redacted>"
  if (Array.isArray(value)) return value.map((entry) => redactValue(entry, retainedSecrets, maxStringLength, key))
  if (value && typeof value === "object") {
    if (value instanceof Error) return redactText(value.stack ?? value.message, retainedSecrets, maxStringLength)
    return Object.fromEntries(Object.entries(value).map(([entryKey, entry]) => [
      entryKey,
      redactValue(entry, retainedSecrets, maxStringLength, entryKey),
    ]))
  }
  if (typeof value !== "string") return value
  return redactText(value, retainedSecrets, maxStringLength)
}

function redactText(value, retainedSecrets, maxStringLength) {
  let result = String(value).replace(CONTROL_CHARACTERS, "")
  for (const secret of retainedSecrets) result = result.split(secret).join("<redacted>")
  for (const pattern of SECRET_TEXT_PATTERNS) {
    result = result.replace(pattern, (_match, prefix) => prefix ? `${prefix}<redacted>` : "<redacted>")
  }
  if (result.length > maxStringLength) result = `<truncated>${result.slice(-maxStringLength)}`
  return result
}

function assertByteBudget(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${label} must be a non-negative safe integer byte count`)
  }
}

function requireSafeToken(value, label, violations) {
  if (!nonEmptyString(value) || /[\s\u0000-\u001f\u007f]/.test(String(value))) {
    violations.push(`${label} must be an exact non-empty token`)
  }
}

function isBroadResourceSelector(value) {
  return value === "*" || value === "all" || value === "--all" || value === "-a" || value.includes("prune")
}

function commandArgs(command) {
  if (Array.isArray(command)) return command.map((entry) => String(entry))
  if (typeof command === "string" && command.trim()) return command.trim().split(/\s+/)
  return null
}

function difference(left, right) {
  return [...left].filter((entry) => !right.has(entry))
}

function safeCounter(value) {
  const candidate = Number(value)
  return Number.isSafeInteger(candidate) && candidate >= 0 ? candidate : null
}

function ratio(available, total) {
  const resolvedAvailable = numeric(available)
  const resolvedTotal = numeric(total)
  if (resolvedTotal <= 0 || resolvedAvailable < 0) throw new Error("resource snapshot contains invalid byte counts")
  return resolvedAvailable / resolvedTotal
}

function numeric(value) {
  return Number.isFinite(Number(value)) ? Number(value) : 0
}

function names(values) {
  if (typeof values === "string") values = values.split("\n")
  if (!Array.isArray(values)) return []
  return values.map((value) => String(value).trim()).filter(Boolean)
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function toIso(value) {
  const resolved = typeof value === "function" ? value() : value
  return new Date(resolved).toISOString()
}

async function resolveMetric(value) {
  const resolved = typeof value === "function" ? await value() : value
  if (resolved === undefined || resolved === null) return undefined
  if (resolved && typeof resolved === "object") return resolved.count ?? resolved.bytes
  return Number(resolved)
}

async function exists(target) {
  try {
    await access(target)
    return true
  } catch {
    return false
  }
}

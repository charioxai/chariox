import { access, statfs } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

const DEFAULT_MIN_MEMORY_HEADROOM = 0.2
const DEFAULT_MIN_DISK_HEADROOM = 0.15
const SLICE_CONTAINER_PREFIX = "chariox-slice-"

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
}) {
  if (typeof runCommand !== "function") throw new Error("runCommand is required")
  if (!nonEmptyString(filesystemPath)) throw new Error("filesystemPath is required")

  const [disk, dockerContainers, dockerVolumes, availableMemoryBytes] = await Promise.all([
    statfs(filesystemPath),
    dockerNames(runCommand, ["ps", "-a", "--format", "{{.Names}}"]),
    dockerNames(runCommand, ["volume", "ls", "--format", "{{.Name}}"]),
    resolveAvailableMemoryBytes(runCommand, platform),
  ])
  const blockSize = Number(disk.bsize)

  return {
    capturedAt: new Date().toISOString(),
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
    },
  }
}

export function evaluateBrowserComputerPreflight(snapshot, options = {}) {
  const minMemoryHeadroom = options.minMemoryHeadroom ?? DEFAULT_MIN_MEMORY_HEADROOM
  const minDiskHeadroom = options.minDiskHeadroom ?? DEFAULT_MIN_DISK_HEADROOM
  const allowExistingHeadedSlices = options.allowExistingHeadedSlices === true
  assertFraction(minMemoryHeadroom, "minMemoryHeadroom")
  assertFraction(minDiskHeadroom, "minDiskHeadroom")

  const memoryHeadroom = ratio(snapshot?.memory?.availableBytes, snapshot?.memory?.totalBytes)
  const diskHeadroom = ratio(snapshot?.disk?.availableBytes, snapshot?.disk?.totalBytes)
  const existingSliceContainers = names(snapshot?.docker?.containers)
    .filter((name) => name.startsWith(SLICE_CONTAINER_PREFIX))
  const violations = []

  if (memoryHeadroom < minMemoryHeadroom) {
    violations.push(`available memory headroom ${(memoryHeadroom * 100).toFixed(1)}% is below ${(minMemoryHeadroom * 100).toFixed(1)}%`)
  }
  if (diskHeadroom < minDiskHeadroom) {
    violations.push(`available disk headroom ${(diskHeadroom * 100).toFixed(1)}% is below ${(minDiskHeadroom * 100).toFixed(1)}%`)
  }
  if (!allowExistingHeadedSlices && existingSliceContainers.length > 0) {
    violations.push(`existing slice containers make a single-slice developer run unsafe: ${existingSliceContainers.join(", ")}`)
  }

  return {
    ok: violations.length === 0,
    memoryHeadroom,
    diskHeadroom,
    existingSliceContainers,
    violations,
  }
}

export function assertBrowserComputerPreflight(snapshot, options = {}) {
  const result = evaluateBrowserComputerPreflight(snapshot, options)
  if (!result.ok) {
    throw new Error(`browser/computer drill resource preflight failed:\n- ${result.violations.join("\n- ")}`)
  }
  return result
}

export async function evaluateBrowserComputerCleanup({
  before,
  after,
  ownedContainers = [],
  ownedVolumes = [],
  tempRoots = [],
  liveChildLabels = [],
}) {
  const afterContainers = new Set(names(after?.docker?.containers))
  const afterVolumes = new Set(names(after?.docker?.volumes))
  const beforeContainers = new Set(names(before?.docker?.containers))
  const beforeVolumes = new Set(names(before?.docker?.volumes))
  const violations = []

  for (const name of names(ownedContainers)) {
    if (afterContainers.has(name)) violations.push(`owned container remains: ${name}`)
  }
  for (const name of names(ownedVolumes)) {
    if (afterVolumes.has(name)) violations.push(`owned volume remains: ${name}`)
  }
  for (const name of afterContainers) {
    if (name.startsWith(SLICE_CONTAINER_PREFIX) && !beforeContainers.has(name)) {
      violations.push(`new slice container remains: ${name}`)
    }
  }
  for (const name of afterVolumes) {
    if (name.startsWith(SLICE_CONTAINER_PREFIX) && !beforeVolumes.has(name)) {
      violations.push(`new slice volume remains: ${name}`)
    }
  }
  for (const tempRoot of tempRoots) {
    if (await exists(tempRoot)) violations.push(`temporary root remains: ${tempRoot}`)
  }
  for (const label of names(liveChildLabels)) {
    violations.push(`child process remains alive: ${label}`)
  }

  return {
    ok: violations.length === 0,
    violations,
    memoryAvailableDeltaBytes: numeric(after?.memory?.availableBytes) - numeric(before?.memory?.availableBytes),
    diskAvailableDeltaBytes: numeric(after?.disk?.availableBytes) - numeric(before?.disk?.availableBytes),
  }
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
  const pageSize = Number(output.match(/page size of (\d+) bytes/i)?.[1])
  if (!Number.isFinite(pageSize) || pageSize <= 0) return null
  const pages = new Map()
  for (const match of output.matchAll(/^Pages ([^:]+):\s+([0-9.]+)\.?$/gm)) {
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

async function exists(target) {
  try {
    await access(target)
    return true
  } catch {
    return false
  }
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
  if (!Array.isArray(values)) return []
  return values.map((value) => String(value).trim()).filter(Boolean)
}

function assertFraction(value, label) {
  if (!Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error(`${label} must be between 0 and 1`)
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

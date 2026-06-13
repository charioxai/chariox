import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

const FAILURE_MANIFEST_FILE = "arroba-drill-failure.json"
const FAILURE_MANIFEST_SCHEMA = "arroba.drill.failure.v1"

export async function readDrillFailureManifest(inputPath) {
  const manifestPath = await resolveFailureManifestPath(inputPath)
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"))
  validateDrillFailureManifest(manifest, manifestPath)
  return manifest
}

export async function findDrillFailureManifestPaths(rootPaths, { maxDepth = 8 } = {}) {
  const roots = Array.isArray(rootPaths) ? rootPaths : [rootPaths]
  const manifests = new Set()
  for (const root of roots) {
    await collectFailureManifestPaths(manifests, root, { depth: 0, maxDepth })
  }
  return [...manifests].sort()
}

export async function resolveFailureManifestPath(inputPath) {
  const inputStat = await stat(inputPath)
  return inputStat.isDirectory() ? path.join(inputPath, FAILURE_MANIFEST_FILE) : inputPath
}

export function validateDrillFailureManifest(manifest, source = "manifest") {
  if (!manifest || typeof manifest !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== FAILURE_MANIFEST_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!nonEmptyString(manifest.rootDir)) {
    throw new Error(`${source} is missing rootDir`)
  }
  if (!nonEmptyString(manifest.failedAt)) {
    throw new Error(`${source} is missing failedAt`)
  }
  if (!manifest.metadata || typeof manifest.metadata !== "object" || Array.isArray(manifest.metadata)) {
    throw new Error(`${source} has invalid metadata`)
  }
  if (manifest.error !== null) {
    validateFailureError(manifest.error, `${source}.error`)
  }
}

export function summarizeDrillFailureManifest(manifest, { source = null } = {}) {
  validateDrillFailureManifest(manifest, source ?? "manifest")
  return {
    schema: manifest.schema,
    source,
    rootDir: manifest.rootDir,
    failedAt: manifest.failedAt,
    metadata: summarizeMetadata(manifest.metadata),
    error: manifest.error
      ? {
          name: manifest.error.name,
          message: manifest.error.message,
          hasStack: typeof manifest.error.stack === "string" && manifest.error.stack.length > 0,
        }
      : null,
  }
}

export function formatDrillFailureManifestSummary(manifest, { source = null } = {}) {
  const summary = summarizeDrillFailureManifest(manifest, { source })
  const drill = summary.metadata.drill ?? "unknown"
  const lines = [
    `drill failure: ${drill}${source ? ` (${source})` : ""}`,
    `root=${summary.rootDir}`,
    `failed_at=${summary.failedAt}`,
  ]
  const metadata = Object.entries(summary.metadata)
    .filter(([key]) => key !== "drill")
    .map(([key, value]) => `${key}=${value}`)
  if (metadata.length > 0) {
    lines.push(`metadata: ${metadata.join(" ")}`)
  }
  if (summary.error) {
    lines.push(`error=${summary.error.name}: ${summary.error.message}`)
    lines.push(`stack=${summary.error.hasStack ? "preserved" : "absent"}`)
  } else {
    lines.push("error=none")
  }
  lines.push(`next: inspect preserved artifacts under ${summary.rootDir}; rerun the drill after addressing the failure`)
  return lines.join("\n")
}

function validateFailureError(error, source) {
  if (!error || typeof error !== "object" || Array.isArray(error)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(error.name)) {
    throw new Error(`${source} is missing name`)
  }
  if (typeof error.message !== "string") {
    throw new Error(`${source} has invalid message`)
  }
  if (error.stack !== null && typeof error.stack !== "string") {
    throw new Error(`${source} has invalid stack`)
  }
}

function summarizeMetadata(metadata) {
  const entries = Object.entries(metadata)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => [key, summarizeMetadataValue(key, value)])
    .filter(([, value]) => value !== null)
  return Object.fromEntries(entries)
}

function summarizeMetadataValue(key, value) {
  if (isSensitiveKey(key)) return "<redacted>"
  if (typeof value === "string") return value.length > 160 ? `${value.slice(0, 157)}...` : value
  if (typeof value === "number" || typeof value === "boolean") return String(value)
  if (value === null) return "null"
  return null
}

function isSensitiveKey(key) {
  return /token|secret|password|credential|cookie|key/i.test(key)
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

async function collectFailureManifestPaths(manifests, currentPath, { depth, maxDepth }) {
  let currentStat = null
  try {
    currentStat = await stat(currentPath)
  } catch {
    return
  }
  if (currentStat.isFile()) {
    if (path.basename(currentPath) === FAILURE_MANIFEST_FILE) manifests.add(currentPath)
    return
  }
  if (!currentStat.isDirectory() || depth > maxDepth) return
  const directManifest = path.join(currentPath, FAILURE_MANIFEST_FILE)
  try {
    const directStat = await stat(directManifest)
    if (directStat.isFile()) manifests.add(directManifest)
  } catch {}

  let dir = null
  try {
    dir = await opendir(currentPath)
    for await (const entry of dir) {
      if (!entry.isDirectory()) {
        if (entry.isFile() && entry.name === FAILURE_MANIFEST_FILE) {
          manifests.add(path.join(currentPath, entry.name))
        }
        continue
      }
      if (shouldPruneDirectory(entry.name)) continue
      await collectFailureManifestPaths(manifests, path.join(currentPath, entry.name), { depth: depth + 1, maxDepth })
    }
  } catch {}
}

function shouldPruneDirectory(name) {
  return name === ".git"
    || name === "node_modules"
    || name === ".pnpm-store"
    || name === "debug"
    || name === "release"
}

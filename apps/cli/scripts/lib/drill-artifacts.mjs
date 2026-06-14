import { mkdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"

export async function prepareDrillArtifacts(rootDir) {
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(rootDir, { recursive: true })
  return rootDir
}

export async function finalizeDrillArtifacts({
  rootDir,
  passed,
  log = null,
  preserveOnFailure = true,
  failure = null,
  metadata = {},
}) {
  if (passed || !preserveOnFailure) {
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    if (!passed && log) {
      log("discarded-failed-run", { rootDir })
    }
    return { preserved: false, rootDir }
  }

  await mkdir(rootDir, { recursive: true }).catch(() => {})
  const manifest = failureManifest({ rootDir, failure, metadata })
  const manifestPath = path.join(rootDir, "arroba-drill-failure.json")
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8").catch(() => {})
  if (log) {
    log("preserved-failed-run", { rootDir, manifestPath })
  }
  return { preserved: true, rootDir, manifestPath }
}

function failureManifest({ rootDir, failure, metadata }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt: new Date().toISOString(),
    metadata: sanitizeManifestMetadata(metadata),
    error: failure
      ? {
          name: failure.name ?? "Error",
          message: failure.message ?? String(failure),
          stack: typeof failure.stack === "string" ? failure.stack : null,
        }
      : null,
  }
}

function sanitizeManifestMetadata(value, key = "") {
  if (isSensitiveMetadataKey(key)) return "<redacted>"
  if (typeof value === "string") {
    return looksLikeSecretValue(value) ? "<redacted>" : value
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return value
  if (Array.isArray(value)) return value.map((item) => sanitizeManifestMetadata(item, key))
  if (!value || typeof value !== "object") return null

  const sanitized = {}
  for (const [childKey, childValue] of Object.entries(value)) {
    sanitized[childKey] = sanitizeManifestMetadata(childValue, childKey)
  }
  return sanitized
}

function isSensitiveMetadataKey(key) {
  return /token|secret|password|credential|cookie|authorization|api[-_]?key/i.test(key)
}

function looksLikeSecretValue(value) {
  return /\bBearer\s+[A-Za-z0-9._~+/=-]{12,}/i.test(value)
    || /\bsk-[A-Za-z0-9_-]{16,}\b/.test(value)
    || /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{16,}\b/.test(value)
}

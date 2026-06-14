import { createHash } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import {
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"

export const DRILL_ARTIFACT_INDEX_SCHEMA = "arroba.drill.artifact_index.v1"

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

export async function writeDrillArtifactIndex({
  rootDir,
  artifacts,
  indexPath = path.join(rootDir, "arroba-drill-artifacts.json"),
  metadata = {},
}) {
  if (!Array.isArray(artifacts) || artifacts.length === 0) {
    throw new Error("drill artifact index requires artifacts")
  }
  const records = []
  for (const artifact of artifacts) {
    records.push(await artifactRecord(rootDir, artifact))
  }
  const index = {
    schema: DRILL_ARTIFACT_INDEX_SCHEMA,
    rootDir,
    createdAt: new Date().toISOString(),
    metadata: sanitizeDrillMetadata(metadata),
    artifacts: records.sort((left, right) => left.path.localeCompare(right.path)),
  }
  validateDrillArtifactIndex(index)
  await mkdir(path.dirname(indexPath), { recursive: true })
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
  return index
}

export async function readDrillArtifactIndex(indexPath) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  validateDrillArtifactIndex(index, indexPath)
  return index
}

export async function verifyDrillArtifactIndex(indexPath) {
  const index = await readDrillArtifactIndex(indexPath)
  for (const artifact of index.artifacts) {
    const contents = await readFile(path.join(index.rootDir, artifact.path))
    if (sha256(contents) !== artifact.sha256) {
      throw new Error(`drill artifact ${artifact.path} sha256 mismatch`)
    }
    if (contents.byteLength !== artifact.sizeBytes) {
      throw new Error(`drill artifact ${artifact.path} sizeBytes mismatch`)
    }
    const schema = schemaForArtifact(contents)
    if (schema !== artifact.schema) {
      throw new Error(`drill artifact ${artifact.path} schema mismatch`)
    }
  }
  return index
}

export function validateDrillArtifactIndex(index, source = "drill artifact index") {
  if (!index || typeof index !== "object" || Array.isArray(index)) {
    throw new Error(`${source} is not an object`)
  }
  if (index.schema !== DRILL_ARTIFACT_INDEX_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(index.schema)}`)
  }
  if (!nonEmptyString(index.rootDir)) {
    throw new Error(`${source} is missing rootDir`)
  }
  if (!nonEmptyString(index.createdAt)) {
    throw new Error(`${source} is missing createdAt`)
  }
  parseDrillIsoTimestamp(index.createdAt, `${source}.createdAt`)
  if (!index.metadata || typeof index.metadata !== "object" || Array.isArray(index.metadata)) {
    throw new Error(`${source} has invalid metadata`)
  }
  if (!Array.isArray(index.artifacts) || index.artifacts.length === 0) {
    throw new Error(`${source} has invalid artifacts`)
  }
  for (const [artifactIndex, artifact] of index.artifacts.entries()) {
    validateArtifactIndexRecord(artifact, `${source}.artifacts[${artifactIndex}]`)
  }
}

function failureManifest({ rootDir, failure, metadata }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt: new Date().toISOString(),
    metadata: sanitizeDrillMetadata(metadata),
    error: failure
      ? {
          name: failure.name ?? "Error",
          message: redactDrillSecretText(failure.message ?? String(failure)),
          stack: typeof failure.stack === "string" ? redactDrillSecretText(failure.stack) : null,
        }
      : null,
  }
}

async function artifactRecord(rootDir, artifact) {
  const input = typeof artifact === "string" ? { path: artifact } : artifact
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("drill artifact entry is not an object")
  }
  const relativePath = relativeArtifactPath(rootDir, input.path)
  const contents = await readFile(path.join(rootDir, relativePath))
  return {
    path: relativePath,
    schema: schemaForArtifact(contents),
    sha256: sha256(contents),
    sizeBytes: contents.byteLength,
  }
}

function validateArtifactIndexRecord(artifact, source) {
  if (!artifact || typeof artifact !== "object" || Array.isArray(artifact)) {
    throw new Error(`${source} is not an object`)
  }
  if (!safeRelativePath(artifact.path)) {
    throw new Error(`${source} has unsafe path ${JSON.stringify(artifact.path)}`)
  }
  if (artifact.schema !== null && !nonEmptyString(artifact.schema)) {
    throw new Error(`${source} has invalid schema`)
  }
  if (typeof artifact.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(artifact.sha256)) {
    throw new Error(`${source} has invalid sha256`)
  }
  if (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0) {
    throw new Error(`${source} has invalid sizeBytes`)
  }
}

function relativeArtifactPath(rootDir, artifactPath) {
  if (!nonEmptyString(rootDir)) throw new Error("drill artifact rootDir is required")
  if (!nonEmptyString(artifactPath)) throw new Error("drill artifact path is required")
  const absoluteRoot = path.resolve(rootDir)
  const absoluteArtifact = path.resolve(absoluteRoot, artifactPath)
  const relativePath = path.relative(absoluteRoot, absoluteArtifact)
  if (!safeRelativePath(relativePath)) {
    throw new Error(`drill artifact path escapes root: ${JSON.stringify(artifactPath)}`)
  }
  return relativePath
}

function safeRelativePath(value) {
  return typeof value === "string"
    && value.length > 0
    && !path.isAbsolute(value)
    && !value.split(/[\\/]/).includes("..")
}

function schemaForArtifact(contents) {
  try {
    const parsed = JSON.parse(contents.toString("utf8"))
    return nonEmptyString(parsed?.schema) ? parsed.schema : null
  } catch {
    return null
  }
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

import { createHash } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"
import { drillRuntimeSignalOwner } from "./drill-runtime-signals.mjs"

export const DRILL_ARTIFACT_INDEX_SCHEMA = "arroba.drill.artifact_index.v1"
export const DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA = "arroba.drill.artifact_index.aggregate.v1"
const DRILL_ARTIFACT_INDEX_FILE = "arroba-drill-artifacts.json"

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
  indexPath = path.join(rootDir, DRILL_ARTIFACT_INDEX_FILE),
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

export async function writeDrillJsonArtifactOutput({
  outputPath,
  value,
  artifactIndexPath = null,
  metadata = {},
}) {
  await mkdir(path.dirname(outputPath), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(value, null, 2)}\n`, "utf8")
  if (!artifactIndexPath) return null
  return await writeDrillArtifactIndex({
    rootDir: path.dirname(outputPath),
    artifacts: [path.basename(outputPath)],
    indexPath: artifactIndexPath,
    metadata,
  })
}

export async function findDrillArtifactIndexPaths(roots, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(roots, {
    fileName: DRILL_ARTIFACT_INDEX_FILE,
    maxDepth,
    schema: DRILL_ARTIFACT_INDEX_SCHEMA,
  })
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

export function summarizeDrillArtifactIndexes(indexes, { sources = [] } = {}) {
  const totals = {
    indexes: indexes.length,
    artifacts: 0,
    sizeBytes: 0,
  }
  const schemas = new Map()
  const runtimeSignals = new Map()
  const runtimeSignalOwners = new Map()
  const owners = new Map()
  const classifications = new Map()
  const summaries = indexes.map((index, indexPosition) => {
    validateDrillArtifactIndex(index, sources[indexPosition] ?? "drill artifact index")
    const indexTotals = {
      artifacts: index.artifacts.length,
      sizeBytes: 0,
    }
    const indexSchemas = new Map()
    const indexRuntimeSignals = runtimeSignalsFromMetadata(index.metadata)
    const indexRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(indexRuntimeSignals)
    const indexOwners = metadataListFromMetadata(index.metadata, "owners")
    const indexClassifications = metadataListFromMetadata(index.metadata, "classifications")
    countValues(indexRuntimeSignals, runtimeSignals)
    countValues(indexRuntimeSignalOwners, runtimeSignalOwners)
    countValues(indexOwners, owners)
    countValues(indexClassifications, classifications)
    for (const artifact of index.artifacts) {
      totals.artifacts += 1
      totals.sizeBytes += artifact.sizeBytes
      indexTotals.sizeBytes += artifact.sizeBytes
      const schema = artifact.schema ?? "none"
      schemas.set(schema, (schemas.get(schema) ?? 0) + 1)
      indexSchemas.set(schema, (indexSchemas.get(schema) ?? 0) + 1)
    }
    return {
      source: sources[indexPosition] ?? null,
      rootDir: index.rootDir,
      artifacts: indexTotals.artifacts,
      sizeBytes: indexTotals.sizeBytes,
      schemas: sortedCountObject(indexSchemas),
      runtimeSignals: countValues(indexRuntimeSignals),
      runtimeSignalOwners: countValues(indexRuntimeSignalOwners),
      owners: countValues(indexOwners),
      classifications: countValues(indexClassifications),
    }
  })
  const aggregate = {
    schema: DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
    totals,
    schemas: sortedCountObject(schemas),
    runtimeSignals: sortedCountObject(runtimeSignals),
    runtimeSignalOwners: sortedCountObject(runtimeSignalOwners),
    owners: sortedCountObject(owners),
    classifications: sortedCountObject(classifications),
    indexes: summaries,
  }
  validateDrillArtifactIndexAggregate(aggregate)
  return aggregate
}

export function diagnosticMetadataForDrillArtifactIndexAggregate(aggregate) {
  return {
    ...(Object.keys(aggregate.runtimeSignals ?? {}).length > 0
      ? { runtimeSignals: Object.keys(aggregate.runtimeSignals).sort().join(",") }
      : {}),
    ...(Object.keys(aggregate.runtimeSignalOwners ?? {}).length > 0
      ? { runtimeSignalOwners: Object.keys(aggregate.runtimeSignalOwners).sort().join(",") }
      : {}),
    ...(Object.keys(aggregate.owners ?? {}).length > 0
      ? { owners: Object.keys(aggregate.owners).sort().join(",") }
      : {}),
    ...(Object.keys(aggregate.classifications ?? {}).length > 0
      ? { classifications: Object.keys(aggregate.classifications).sort().join(",") }
      : {}),
  }
}

export function formatDrillArtifactIndexAggregateSummary(aggregate) {
  validateDrillArtifactIndexAggregate(aggregate)
  const lines = [
    "drill artifact index aggregate:",
    `indexes=${aggregate.totals.indexes} artifacts=${aggregate.totals.artifacts} size_bytes=${aggregate.totals.sizeBytes}`,
  ]
  const schemas = Object.entries(aggregate.schemas)
  if (schemas.length > 0) {
    lines.push(`schemas: ${schemas.map(([schema, count]) => `${schema}=${count}`).join(" ")}`)
  }
  const runtimeSignals = Object.entries(aggregate.runtimeSignals ?? {})
  if (runtimeSignals.length > 0) {
    lines.push(`runtime_signals: ${runtimeSignals.map(([signal, count]) => `${signal}=${count}`).join(" ")}`)
  }
  const runtimeSignalOwners = Object.entries(aggregate.runtimeSignalOwners ?? {})
  if (runtimeSignalOwners.length > 0) {
    lines.push(`runtime_signal_owners: ${runtimeSignalOwners.map(([owner, count]) => `${owner}=${count}`).join(" ")}`)
  }
  const owners = Object.entries(aggregate.owners ?? {})
  if (owners.length > 0) {
    lines.push(`owners: ${owners.map(([owner, count]) => `${owner}=${count}`).join(" ")}`)
  }
  const classifications = Object.entries(aggregate.classifications ?? {})
  if (classifications.length > 0) {
    lines.push(`classifications: ${classifications.map(([classification, count]) => `${classification}=${count}`).join(" ")}`)
  }
  lines.push("next: verify indexed artifacts before using them as validation evidence")
  return lines.join("\n")
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

export function validateDrillArtifactIndexAggregate(aggregate, source = "drill artifact index aggregate") {
  if (!aggregate || typeof aggregate !== "object" || Array.isArray(aggregate)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate.schema !== DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  if (!aggregate.totals || typeof aggregate.totals !== "object" || Array.isArray(aggregate.totals)) {
    throw new Error(`${source} has invalid totals`)
  }
  for (const key of ["indexes", "artifacts", "sizeBytes"]) {
    if (!Number.isSafeInteger(aggregate.totals[key]) || aggregate.totals[key] < 0) {
      throw new Error(`${source}.totals has invalid ${key}`)
    }
  }
  validateCountObject(aggregate.schemas, `${source}.schemas`)
  validateCountObject(aggregate.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateCountObject(aggregate.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(aggregate.owners ?? {}, `${source}.owners`)
  validateCountObject(aggregate.classifications ?? {}, `${source}.classifications`)
  if (!Array.isArray(aggregate.indexes)) {
    throw new Error(`${source} has invalid indexes`)
  }
  for (const [index, summary] of aggregate.indexes.entries()) {
    validateArtifactIndexSummary(summary, `${source}.indexes[${index}]`)
  }
  if (aggregate.totals.indexes !== aggregate.indexes.length) {
    throw new Error(`${source} totals.indexes does not match indexes`)
  }
  const expectedArtifacts = aggregate.indexes.reduce((sum, index) => sum + index.artifacts, 0)
  const expectedSizeBytes = aggregate.indexes.reduce((sum, index) => sum + index.sizeBytes, 0)
  const expectedRuntimeSignals = new Map()
  const expectedRuntimeSignalOwners = new Map()
  const expectedOwners = new Map()
  const expectedClassifications = new Map()
  for (const index of aggregate.indexes) {
    for (const [signal, count] of Object.entries(index.runtimeSignals ?? {})) {
      expectedRuntimeSignals.set(signal, (expectedRuntimeSignals.get(signal) ?? 0) + count)
    }
    for (const [owner, count] of Object.entries(index.runtimeSignalOwners ?? {})) {
      expectedRuntimeSignalOwners.set(owner, (expectedRuntimeSignalOwners.get(owner) ?? 0) + count)
    }
    for (const [owner, count] of Object.entries(index.owners ?? {})) {
      expectedOwners.set(owner, (expectedOwners.get(owner) ?? 0) + count)
    }
    for (const [classification, count] of Object.entries(index.classifications ?? {})) {
      expectedClassifications.set(classification, (expectedClassifications.get(classification) ?? 0) + count)
    }
  }
  if (aggregate.totals.artifacts !== expectedArtifacts || aggregate.totals.sizeBytes !== expectedSizeBytes) {
    throw new Error(`${source} totals do not match indexes`)
  }
  if (JSON.stringify(aggregate.runtimeSignals ?? {}) !== JSON.stringify(sortedCountObject(expectedRuntimeSignals))) {
    throw new Error(`${source} runtimeSignals do not match indexes`)
  }
  if (JSON.stringify(aggregate.runtimeSignalOwners ?? {}) !== JSON.stringify(sortedCountObject(expectedRuntimeSignalOwners))) {
    throw new Error(`${source} runtimeSignalOwners do not match indexes`)
  }
  if (JSON.stringify(aggregate.owners ?? {}) !== JSON.stringify(sortedCountObject(expectedOwners))) {
    throw new Error(`${source} owners do not match indexes`)
  }
  if (JSON.stringify(aggregate.classifications ?? {}) !== JSON.stringify(sortedCountObject(expectedClassifications))) {
    throw new Error(`${source} classifications do not match indexes`)
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

function validateArtifactIndexSummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  if (summary.source !== null && typeof summary.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(summary.rootDir)) {
    throw new Error(`${source} is missing rootDir`)
  }
  for (const key of ["artifacts", "sizeBytes"]) {
    if (!Number.isSafeInteger(summary[key]) || summary[key] < 0) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
  validateCountObject(summary.schemas, `${source}.schemas`)
  validateCountObject(summary.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateCountObject(summary.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(summary.owners ?? {}, `${source}.owners`)
  validateCountObject(summary.classifications ?? {}, `${source}.classifications`)
}

function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count`)
    }
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

function sortedCountObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function runtimeSignalsFromMetadata(metadata) {
  return metadataListFromMetadata(metadata, "runtimeSignals")
}

function runtimeSignalOwnersFromRuntimeSignals(runtimeSignals) {
  return [...new Set(runtimeSignals.map((signal) => drillRuntimeSignalOwner(signal)))].sort()
}

function metadataListFromMetadata(metadata, key) {
  const value = metadata?.[key]
  if (typeof value !== "string") return []
  return [...new Set(value.split(",").map((item) => item.trim()).filter(nonEmptyString))].sort()
}

function countValues(values, target = new Map()) {
  for (const value of values) {
    target.set(value, (target.get(value) ?? 0) + 1)
  }
  return sortedCountObject(target)
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

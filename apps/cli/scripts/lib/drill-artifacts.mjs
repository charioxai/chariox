import { createHash } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { isKnownDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { isKnownDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { isKnownDrillGeneratedEvidenceKind } from "./drill-generated-evidence-kinds.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"
import {
  drillRuntimeSignalOwnersFor,
  isKnownDrillRuntimeSignal,
  validateDrillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"

export const DRILL_ARTIFACT_INDEX_SCHEMA = "arroba.drill.artifact_index.v1"
export const DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA = "arroba.drill.artifact_index.aggregate.v1"
export const DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS = Object.freeze([
  "runtimeSignals",
  "runtimeSignalOwners",
  "coverageAreas",
  "owners",
  "classifications",
  "artifactKinds",
  "generatedEvidenceKinds",
  "requiredGeneratedEvidenceKinds",
  "missingGeneratedEvidenceKinds",
  "evidenceRepos",
])
export const DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS = Object.freeze([
  "schemas",
  ...DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
])
const DRILL_ARTIFACT_INDEX_FILE = "arroba-drill-artifacts.json"
const DRILL_ARTIFACT_DIAGNOSTIC_LABELS = Object.freeze({
  runtimeSignals: "runtime_signals",
  runtimeSignalOwners: "runtime_signal_owners",
  coverageAreas: "coverage_areas",
  owners: "owners",
  classifications: "classifications",
  artifactKinds: "artifact_kinds",
  generatedEvidenceKinds: "generated_evidence_kinds",
  requiredGeneratedEvidenceKinds: "required_generated_evidence_kinds",
  missingGeneratedEvidenceKinds: "missing_generated_evidence_kinds",
  evidenceRepos: "evidence_repos",
})

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
    validateKnownArtifactContents(contents, artifact.path, index.metadata)
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
  const coverageAreas = new Map()
  const owners = new Map()
  const classifications = new Map()
  const artifactKinds = new Map()
  const generatedEvidenceKinds = new Map()
  const requiredGeneratedEvidenceKinds = new Map()
  const missingGeneratedEvidenceKinds = new Map()
  const evidenceRepos = new Map()
  const summaries = indexes.map((index, indexPosition) => {
    validateDrillArtifactIndex(index, sources[indexPosition] ?? "drill artifact index")
    const indexTotals = {
      artifacts: index.artifacts.length,
      sizeBytes: 0,
    }
    const indexSchemas = new Map()
    const indexRuntimeSignals = runtimeSignalsFromMetadata(index.metadata)
    const indexRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(indexRuntimeSignals)
    const indexCoverageAreas = metadataListFromMetadata(index.metadata, "coverageAreas")
    const indexOwners = metadataListFromMetadata(index.metadata, "owners")
    const indexClassifications = metadataListFromMetadata(index.metadata, "classifications")
    const indexArtifactKinds = metadataListFromMetadata(index.metadata, "artifactKinds")
    const indexGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "generatedEvidenceKinds")
    const indexRequiredGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "requiredGeneratedEvidenceKinds")
    const indexMissingGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "missingGeneratedEvidenceKinds")
    const indexEvidenceRepos = metadataListFromMetadata(index.metadata, "evidenceRepos")
    countValues(indexRuntimeSignals, runtimeSignals)
    countValues(indexRuntimeSignalOwners, runtimeSignalOwners)
    countValues(indexCoverageAreas, coverageAreas)
    countValues(indexOwners, owners)
    countValues(indexClassifications, classifications)
    countValues(indexArtifactKinds, artifactKinds)
    countValues(indexGeneratedEvidenceKinds, generatedEvidenceKinds)
    countValues(indexRequiredGeneratedEvidenceKinds, requiredGeneratedEvidenceKinds)
    countValues(indexMissingGeneratedEvidenceKinds, missingGeneratedEvidenceKinds)
    countValues(indexEvidenceRepos, evidenceRepos)
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
      coverageAreas: countValues(indexCoverageAreas),
      owners: countValues(indexOwners),
      classifications: countValues(indexClassifications),
      artifactKinds: countValues(indexArtifactKinds),
      generatedEvidenceKinds: countValues(indexGeneratedEvidenceKinds),
      requiredGeneratedEvidenceKinds: countValues(indexRequiredGeneratedEvidenceKinds),
      missingGeneratedEvidenceKinds: countValues(indexMissingGeneratedEvidenceKinds),
      evidenceRepos: countValues(indexEvidenceRepos),
    }
  })
  const aggregate = {
    schema: DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
    totals,
    schemas: sortedCountObject(schemas),
    runtimeSignals: sortedCountObject(runtimeSignals),
    runtimeSignalOwners: sortedCountObject(runtimeSignalOwners),
    coverageAreas: sortedCountObject(coverageAreas),
    owners: sortedCountObject(owners),
    classifications: sortedCountObject(classifications),
    artifactKinds: sortedCountObject(artifactKinds),
    generatedEvidenceKinds: sortedCountObject(generatedEvidenceKinds),
    requiredGeneratedEvidenceKinds: sortedCountObject(requiredGeneratedEvidenceKinds),
    missingGeneratedEvidenceKinds: sortedCountObject(missingGeneratedEvidenceKinds),
    evidenceRepos: sortedCountObject(evidenceRepos),
    indexes: summaries,
  }
  validateDrillArtifactIndexAggregate(aggregate)
  return aggregate
}

export function diagnosticMetadataForDrillArtifactIndexAggregate(aggregate) {
  validateDrillArtifactDiagnosticDimensions(aggregate)
  return Object.fromEntries(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS
    .map((key) => [key, Object.keys(aggregate[key] ?? {}).sort().join(",")])
    .filter(([, value]) => value.length > 0))
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
  for (const key of DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS) {
    const entries = Object.entries(aggregate[key] ?? {})
    if (entries.length > 0) {
      lines.push(`${DRILL_ARTIFACT_DIAGNOSTIC_LABELS[key]}: ${entries.map(([name, count]) => `${name}=${count}`).join(" ")}`)
    }
  }
  lines.push("next: verify indexed artifacts before using them as validation evidence")
  return lines.join("\n")
}

export function validateDrillArtifactDiagnosticDimensions(value, source = "drill artifact diagnostics") {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS) {
    if (!Object.hasOwn(value, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(value[key], `${source}.${key}`, key)
  }
  validateRuntimeSignalOwnerKeysMatch(value.runtimeSignals, value.runtimeSignalOwners, source)
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
  validateDrillArtifactIndexKindMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexGeneratedEvidenceMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexEvidenceRepoMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexRuntimeSignalOwnerMetadata(index.metadata, `${source}.metadata`)
  if (!Array.isArray(index.artifacts) || index.artifacts.length === 0) {
    throw new Error(`${source} has invalid artifacts`)
  }
  const seen = new Set()
  for (const [artifactIndex, artifact] of index.artifacts.entries()) {
    validateArtifactIndexRecord(artifact, `${source}.artifacts[${artifactIndex}]`)
    if (seen.has(artifact.path)) {
      throw new Error(`${source} has duplicate artifact ${artifact.path}`)
    }
    seen.add(artifact.path)
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
  for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
    if (!Object.hasOwn(aggregate, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(aggregate[key], `${source}.${key}`, key)
  }
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
  const expectedAggregateCounts = Object.fromEntries(
    DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS.map((key) => [key, new Map()]),
  )
  for (const index of aggregate.indexes) {
    for (const [key, expectedCounts] of Object.entries(expectedAggregateCounts)) {
      for (const [value, count] of Object.entries(index[key] ?? {})) {
        expectedCounts.set(value, (expectedCounts.get(value) ?? 0) + count)
      }
    }
  }
  if (aggregate.totals.artifacts !== expectedArtifacts || aggregate.totals.sizeBytes !== expectedSizeBytes) {
    throw new Error(`${source} totals do not match indexes`)
  }
  for (const [key, expectedCounts] of Object.entries(expectedAggregateCounts)) {
    if (JSON.stringify(aggregate[key] ?? {}) !== JSON.stringify(sortedCountObject(expectedCounts))) {
      throw new Error(`${source} ${key} do not match indexes`)
    }
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
  for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
    if (!Object.hasOwn(summary, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(summary[key], `${source}.${key}`, key)
  }
  validateRuntimeSignalOwnerCountsMatch(summary.runtimeSignals, summary.runtimeSignalOwners, source)
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

function validateDiagnosticCountObject(value, source, key) {
  validateCountObject(value, source)
  if (key === "artifactKinds") {
    for (const kind of Object.keys(value)) {
      if (!isKnownDrillArtifactKind(kind)) {
        throw new Error(`${source} has unknown artifact kind ${JSON.stringify(kind)}`)
      }
    }
  }
  if ([
    "generatedEvidenceKinds",
    "requiredGeneratedEvidenceKinds",
    "missingGeneratedEvidenceKinds",
  ].includes(key)) {
    for (const kind of Object.keys(value)) {
      if (!isKnownDrillGeneratedEvidenceKind(kind)) {
        throw new Error(`${source} has unknown generated evidence kind ${JSON.stringify(kind)}`)
      }
    }
  }
  if (key === "runtimeSignals") {
    for (const signal of Object.keys(value)) {
      if (!isKnownDrillRuntimeSignal(signal)) {
        throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
      }
    }
  }
  if (key === "evidenceRepos") {
    for (const repo of Object.keys(value)) {
      if (!isKnownDrillArtifactEvidenceRepo(repo)) {
        throw new Error(`${source} has unknown evidence repo ${JSON.stringify(repo)}`)
      }
    }
  }
}

function validateRuntimeSignalOwnerCountsMatch(runtimeSignals, runtimeSignalOwners, source) {
  const expectedOwners = Object.fromEntries(
    drillRuntimeSignalOwnersFor(Object.keys(runtimeSignals ?? {})).map((owner) => [owner, 1]),
  )
  if (JSON.stringify(runtimeSignalOwners ?? {}) !== JSON.stringify(expectedOwners)) {
    throw new Error(`${source}.runtimeSignalOwners must match runtimeSignals`)
  }
}

function validateRuntimeSignalOwnerKeysMatch(runtimeSignals, runtimeSignalOwners, source) {
  validateDiagnosticCountObject(runtimeSignals ?? {}, `${source}.runtimeSignals`, "runtimeSignals")
  validateCountObject(runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  const actualOwners = Object.keys(runtimeSignalOwners ?? {}).sort()
  const expectedOwners = drillRuntimeSignalOwnersFor(Object.keys(runtimeSignals ?? {})).sort()
  if (JSON.stringify(actualOwners) !== JSON.stringify(expectedOwners)) {
    throw new Error(`${source}.runtimeSignalOwners must match runtimeSignals`)
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

function validateKnownArtifactContents(contents, artifactPath, metadata = {}) {
  let parsed
  try {
    parsed = JSON.parse(contents.toString("utf8"))
  } catch {
    return
  }
  const requiresRuntimeSignalManifest = runtimeSignalsFromMetadata(metadata).length > 0
  if (parsed?.schema === "arroba.drill.validation_suite.v1") {
    validateValidationSuiteManifestArtifact(parsed, artifactPath)
    validateValidationSuiteArtifactMetadata({
      artifactPath,
      expectedKind: "validation-suite",
      metadata,
      testCount: parsed.testCount,
    })
    if (requiresRuntimeSignalManifest && parsed.runtimeSignalsManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing runtimeSignalsManifest`)
    }
    if (parsed.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.runtimeSignalsManifest, `${artifactPath}.runtimeSignalsManifest`)
    }
  }
  if (parsed?.schema === "arroba.drill.validation_suite_run.v1") {
    validateValidationSuiteRunArtifact(parsed, artifactPath)
    validateValidationSuiteArtifactMetadata({
      artifactPath,
      expectedKind: "validation-suite-run",
      metadata,
      status: parsed.status,
      testCount: parsed.manifest.testCount,
    })
    if (requiresRuntimeSignalManifest && parsed.manifest?.runtimeSignalsManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing manifest.runtimeSignalsManifest`)
    }
    if (parsed.manifest?.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.manifest.runtimeSignalsManifest, `${artifactPath}.manifest.runtimeSignalsManifest`)
    }
  }
}

function validateValidationSuiteArtifactMetadata({
  artifactPath,
  expectedKind,
  metadata,
  status = null,
  testCount,
}) {
  const artifactKinds = metadataListFromMetadata(metadata, "artifactKinds")
  if (artifactKinds.length > 0 && !artifactKinds.includes(expectedKind)) {
    throw new Error(`drill artifact ${artifactPath} metadata.artifactKinds must include ${expectedKind}`)
  }
  if (metadata?.status !== undefined && metadata.status !== status) {
    throw new Error(`drill artifact ${artifactPath} metadata.status must match artifact status`)
  }
  if (metadata?.tests !== undefined && metadata.tests !== testCount) {
    throw new Error(`drill artifact ${artifactPath} metadata.tests must match artifact testCount`)
  }
}

function validateValidationSuiteRunArtifact(run, source) {
  if (!["passed", "failed"].includes(run.status)) {
    throw new Error(`drill artifact ${source} has invalid status`)
  }
  if (typeof run.ok !== "boolean") {
    throw new Error(`drill artifact ${source} is missing ok`)
  }
  if (run.ok !== (run.status === "passed")) {
    throw new Error(`drill artifact ${source} ok does not match status`)
  }
  const startedMs = parseDrillIsoTimestamp(run.startedAt, `drill artifact ${source}.startedAt`)
  const completedMs = parseDrillIsoTimestamp(run.completedAt, `drill artifact ${source}.completedAt`)
  if (completedMs < startedMs) {
    throw new Error(`drill artifact ${source}.completedAt must not be before startedAt`)
  }
  if (!Number.isSafeInteger(run.durationMs) || run.durationMs < 0) {
    throw new Error(`drill artifact ${source} has invalid durationMs`)
  }
  if (run.durationMs !== completedMs - startedMs) {
    throw new Error(`drill artifact ${source}.durationMs must match completedAt - startedAt`)
  }
  if (run.exitCode !== null && (!Number.isSafeInteger(run.exitCode) || run.exitCode < 0)) {
    throw new Error(`drill artifact ${source} has invalid exitCode`)
  }
  if (run.signal !== null && !nonEmptyString(run.signal)) {
    throw new Error(`drill artifact ${source} has invalid signal`)
  }
  if (run.error !== null && typeof run.error !== "string") {
    throw new Error(`drill artifact ${source} has invalid error`)
  }
  if (run.status === "passed" && (run.exitCode !== 0 || run.signal !== null || run.error !== null)) {
    throw new Error(`drill artifact ${source} passed run has failure fields`)
  }
  validateValidationSuiteManifestArtifact(run.manifest, `${source}.manifest`)
  if (!nonEmptyString(run.command) || run.command !== run.manifest.command) {
    throw new Error(`drill artifact ${source}.command must match manifest.command`)
  }
  if (run.testCount !== run.manifest.testCount) {
    throw new Error(`drill artifact ${source}.testCount must match manifest.testCount`)
  }
  if (!Array.isArray(run.testPaths) || JSON.stringify(run.testPaths) !== JSON.stringify(run.manifest.testPaths)) {
    throw new Error(`drill artifact ${source}.testPaths must match manifest.testPaths`)
  }
}

function validateValidationSuiteManifestArtifact(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`drill artifact ${source} is not an object`)
  }
  if (manifest.schema !== "arroba.drill.validation_suite.v1") {
    throw new Error(`drill artifact ${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!nonEmptyString(manifest.command)) {
    throw new Error(`drill artifact ${source} is missing command`)
  }
  if (!Number.isSafeInteger(manifest.testCount) || manifest.testCount <= 0) {
    throw new Error(`drill artifact ${source} has invalid testCount`)
  }
  if (!Array.isArray(manifest.testPaths) || manifest.testPaths.length !== manifest.testCount) {
    throw new Error(`drill artifact ${source}.testPaths must match testCount`)
  }
  for (const [index, testPath] of manifest.testPaths.entries()) {
    if (!nonEmptyString(testPath)) {
      throw new Error(`drill artifact ${source}.testPaths[${index}] has invalid path`)
    }
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
  return drillRuntimeSignalOwnersFor(runtimeSignals)
}

function validateDrillArtifactIndexRuntimeSignalOwnerMetadata(metadata, source) {
  const runtimeSignals = runtimeSignalsFromMetadata(metadata)
  const runtimeSignalOwners = metadataListFromMetadata(metadata, "runtimeSignalOwners")
  if (runtimeSignals.length === 0 && runtimeSignalOwners.length === 0) return
  const expectedRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(runtimeSignals)
  if (runtimeSignals.length === 0) {
    throw new Error(`${source}.runtimeSignalOwners requires runtimeSignals`)
  }
  if (JSON.stringify(runtimeSignalOwners) !== JSON.stringify(expectedRuntimeSignalOwners)) {
    throw new Error(`${source}.runtimeSignalOwners must match runtimeSignals`)
  }
}

function validateDrillArtifactIndexEvidenceRepoMetadata(metadata, source) {
  for (const repo of metadataListFromMetadata(metadata, "evidenceRepos")) {
    if (!isKnownDrillArtifactEvidenceRepo(repo)) {
      throw new Error(`${source}.evidenceRepos has unknown evidence repo ${JSON.stringify(repo)}`)
    }
  }
}

function validateDrillArtifactIndexGeneratedEvidenceMetadata(metadata, source) {
  for (const key of [
    "generatedEvidenceKinds",
    "requiredGeneratedEvidenceKinds",
    "missingGeneratedEvidenceKinds",
  ]) {
    for (const kind of metadataListFromMetadata(metadata, key)) {
      if (!isKnownDrillGeneratedEvidenceKind(kind)) {
        throw new Error(`${source}.${key} has unknown generated evidence kind ${JSON.stringify(kind)}`)
      }
    }
  }
}

function validateDrillArtifactIndexKindMetadata(metadata, source) {
  for (const kind of metadataListFromMetadata(metadata, "artifactKinds")) {
    if (!isKnownDrillArtifactKind(kind)) {
      throw new Error(`${source}.artifactKinds has unknown artifact kind ${JSON.stringify(kind)}`)
    }
  }
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

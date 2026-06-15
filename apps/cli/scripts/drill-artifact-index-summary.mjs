#!/usr/bin/env node
import { readFile } from "node:fs/promises"
import path from "node:path"
import {
  parseDrillMaxDepth,
  parseDrillNonNegativeInteger,
} from "./lib/drill-cli-args.mjs"
import {
  diagnosticMetadataForDrillArtifactIndexAggregate,
  findDrillArtifactIndexPaths,
  formatDrillArtifactIndexAggregateSummary,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
} from "./lib/drill-artifacts.mjs"
import {
  isKnownDrillProvider,
  parseProviderAccountAlias,
} from "./lib/drill-provider-profiles.mjs"
import { isKnownDrillGeneratedEvidenceKind } from "./lib/drill-generated-evidence-kinds.mjs"
import { isKnownDrillGeneratedMatrixLimitation } from "./lib/drill-generated-matrix-limitations.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-artifact-index-summary.mjs [options]",
    "",
    "Verifies and aggregates drill artifact indexes.",
    "",
    "Options:",
    "  --artifact-index PATH  Read and verify a specific artifact index; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --require-artifact-max-age-ms MS",
    "                         Exit non-zero when artifact indexes are older than this many milliseconds",
    "  --require-matrix-max-age-ms MS",
    "                         Exit non-zero when indexed matrix reports are older than this many milliseconds",
    "  --require-generated-evidence-kind KIND",
    "                         Exit non-zero when generated evidence kind metadata is missing; repeatable",
    "  --require-generated-matrix-limitation KIND",
    "                         Exit non-zero when generated matrix limitation metadata is missing; repeatable",
    "  --require-generated-validation-suite-failure-root PATH",
    "                         Exit non-zero when generated validation-suite failure-root metadata is missing; repeatable",
    "  --require-generated-matrix-artifact-index PATH",
    "                         Exit non-zero when generated matrix artifact-index metadata is missing; repeatable",
    "  --require-provider-account-alias P=A",
    "                         Exit non-zero when provider account alias metadata is missing; repeatable",
    "  --json                 Print aggregate JSON",
    "  --output PATH          Write aggregate JSON to PATH",
    "  --output-artifact-index PATH",
    "                         Write an artifact index for --output",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const discovered = options.artifactRoots.length > 0
    ? await findDrillArtifactIndexPaths(options.artifactRoots, { maxDepth: options.maxDepth })
    : []
  const indexPaths = [...new Set([...options.artifactIndexes, ...discovered])].sort()
  if (indexPaths.length === 0) {
    throw new Error("no drill artifact indexes found")
  }
  const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
  const aggregate = {
    ...summarizeDrillArtifactIndexes(indexes, { sources: indexPaths }),
    ...freshnessDiagnosticsFor(indexes, indexPaths, options),
    ...await matrixFreshnessDiagnosticsFor(indexes, options),
  }
  Object.assign(aggregate, generatedEvidenceKindDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedMatrixLimitationDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedValidationSuiteFailureRootDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedMatrixArtifactIndexDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, providerAccountAliasDiagnosticsFor(aggregate, options))
  if (options.outputPath) {
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: aggregate,
      metadata: {
        drill: "artifact-index-summary",
        indexes: aggregate.totals.indexes,
        ...artifactIndexSummaryOutputMetadataFor(aggregate),
        ...generatedEvidenceKindRequirementMetadataFor(aggregate),
        ...generatedMatrixLimitationRequirementMetadataFor(aggregate),
        ...generatedValidationSuiteFailureRootRequirementMetadataFor(aggregate),
        ...generatedMatrixArtifactIndexRequirementMetadataFor(aggregate),
        ...providerAccountAliasRequirementMetadataFor(aggregate),
      },
    })
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
  } else {
    console.log(formatAggregateSummaryWithFreshness(aggregate))
  }
  if (
    (aggregate.staleArtifactIndexes ?? []).length > 0
    || (aggregate.staleMatrixReports ?? []).length > 0
    || (aggregate.missingRequiredGeneratedEvidenceKinds ?? []).length > 0
    || (aggregate.missingRequiredGeneratedMatrixLimitations ?? []).length > 0
    || (aggregate.missingGeneratedValidationSuiteFailureRoots ?? []).length > 0
    || (aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []).length > 0
    || (aggregate.missingProviderAccountAliases ?? []).length > 0
  ) {
    process.exitCode = 1
  }
}

function parseArgs(argv) {
  const options = {
    artifactIndexes: [],
    artifactRoots: [],
    help: false,
    json: false,
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    requiredArtifactMaxAgeMs: null,
    requiredGeneratedEvidenceKinds: [],
    requiredGeneratedMatrixLimitations: [],
    requiredGeneratedValidationSuiteFailureRoots: [],
    requiredGeneratedMatrixArtifactIndexes: [],
    requiredMatrixMaxAgeMs: null,
    requiredProviderAccountAliases: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--artifact-index") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--artifact-index requires a value")
      options.artifactIndexes.push(value)
      index += 1
    } else if (arg.startsWith("--artifact-index=")) {
      options.artifactIndexes.push(arg.slice("--artifact-index=".length))
    } else if (arg === "--artifact-root") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--artifact-root requires a value")
      options.artifactRoots.push(value)
      index += 1
    } else if (arg.startsWith("--artifact-root=")) {
      options.artifactRoots.push(arg.slice("--artifact-root=".length))
    } else if (arg === "--max-depth") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--max-depth requires a value")
      options.maxDepth = parseDrillMaxDepth(value)
      index += 1
    } else if (arg.startsWith("--max-depth=")) {
      options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
    } else if (arg === "--require-artifact-max-age-ms") {
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-artifact-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-artifact-max-age-ms=")) {
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(
        arg.slice("--require-artifact-max-age-ms=".length),
        "--require-artifact-max-age-ms",
      )
    } else if (arg === "--require-matrix-max-age-ms") {
      options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-matrix-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-matrix-max-age-ms=")) {
      options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(
        arg.slice("--require-matrix-max-age-ms=".length),
        "--require-matrix-max-age-ms",
      )
    } else if (arg === "--require-generated-evidence-kind") {
      options.requiredGeneratedEvidenceKinds.push(parseGeneratedEvidenceKindRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-evidence-kind=")) {
      options.requiredGeneratedEvidenceKinds.push(parseGeneratedEvidenceKindRequirement(
        arg.slice("--require-generated-evidence-kind=".length),
        "--require-generated-evidence-kind",
      ))
    } else if (arg === "--require-generated-matrix-limitation") {
      options.requiredGeneratedMatrixLimitations.push(parseGeneratedMatrixLimitationRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-matrix-limitation=")) {
      options.requiredGeneratedMatrixLimitations.push(parseGeneratedMatrixLimitationRequirement(
        arg.slice("--require-generated-matrix-limitation=".length),
        "--require-generated-matrix-limitation",
      ))
    } else if (arg === "--require-generated-validation-suite-failure-root") {
      options.requiredGeneratedValidationSuiteFailureRoots.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--require-generated-validation-suite-failure-root=")) {
      options.requiredGeneratedValidationSuiteFailureRoots.push(arg.slice("--require-generated-validation-suite-failure-root=".length))
    } else if (arg === "--require-generated-matrix-artifact-index") {
      options.requiredGeneratedMatrixArtifactIndexes.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--require-generated-matrix-artifact-index=")) {
      options.requiredGeneratedMatrixArtifactIndexes.push(arg.slice("--require-generated-matrix-artifact-index=".length))
    } else if (arg === "--require-provider-account-alias") {
      options.requiredProviderAccountAliases.push(parseProviderAccountAliasRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-provider-account-alias=")) {
      options.requiredProviderAccountAliases.push(parseProviderAccountAliasRequirement(
        arg.slice("--require-provider-account-alias=".length),
        "--require-provider-account-alias",
      ))
    } else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    } else if (arg === "--output-artifact-index") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output-artifact-index requires a value")
      options.outputArtifactIndexPath = value
      index += 1
    } else if (arg.startsWith("--output-artifact-index=")) {
      options.outputArtifactIndexPath = arg.slice("--output-artifact-index=".length)
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown argument: ${arg}`)
    } else {
      throw new Error(`unexpected argument: ${arg}`)
    }
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function freshnessDiagnosticsFor(indexes, sources, options) {
  if (options.requiredArtifactMaxAgeMs === null) return {}
  return {
    requiredArtifactMaxAgeMs: options.requiredArtifactMaxAgeMs,
    staleArtifactIndexes: indexes
      .map((index, position) => {
        const createdMs = Date.parse(index.createdAt)
        return {
          source: sources[position] ?? null,
          createdAt: index.createdAt,
          ageMs: Math.max(0, Math.floor(Date.now() - createdMs)),
          maxAgeMs: options.requiredArtifactMaxAgeMs,
        }
      })
      .filter((entry) => entry.ageMs > options.requiredArtifactMaxAgeMs),
  }
}

function parseProviderAccountAliasRequirement(value, flag) {
  try {
    const { provider, alias } = parseProviderAccountAlias(value)
    if (!isKnownDrillProvider(provider)) {
      throw new Error(`unknown provider account alias provider: ${provider}`)
    }
    return `${provider}=${alias}`
  } catch (error) {
    throw new Error(`${flag} has invalid value: ${error.message}`)
  }
}

function parseGeneratedEvidenceKindRequirement(value, flag) {
  if (!isKnownDrillGeneratedEvidenceKind(value)) {
    throw new Error(`${flag} has unknown generated evidence kind: ${value}`)
  }
  return value
}

function parseGeneratedMatrixLimitationRequirement(value, flag) {
  if (!isKnownDrillGeneratedMatrixLimitation(value)) {
    throw new Error(`${flag} has unknown generated matrix limitation: ${value}`)
  }
  return value
}

function artifactIndexSummaryOutputMetadataFor(aggregate) {
  const metadata = diagnosticMetadataForDrillArtifactIndexAggregate(aggregate)
  const artifactKinds = new Set(String(metadata.artifactKinds ?? "").split(",").filter(Boolean))
  artifactKinds.add("artifact-index-aggregate")
  return {
    ...metadata,
    artifactKinds: [...artifactKinds].sort().join(","),
  }
}

async function matrixFreshnessDiagnosticsFor(indexes, options) {
  if (options.requiredMatrixMaxAgeMs === null) return {}
  const staleMatrixReports = []
  for (const index of indexes) {
    for (const artifact of index.artifacts) {
      if (artifact.schema !== "arroba.drill.matrix.v1") continue
      const source = path.join(index.rootDir, artifact.path)
      const report = JSON.parse(await readFile(source, "utf8"))
      const completedMs = Date.parse(report.completedAt)
      if (!Number.isFinite(completedMs)) {
        throw new Error(`indexed matrix report ${source} has invalid completedAt`)
      }
      const ageMs = Math.max(0, Math.floor(Date.now() - completedMs))
      if (ageMs > options.requiredMatrixMaxAgeMs) {
        staleMatrixReports.push({
          source,
          matrix: report.matrix,
          completedAt: report.completedAt,
          ageMs,
          maxAgeMs: options.requiredMatrixMaxAgeMs,
        })
      }
    }
  }
  return {
    requiredMatrixMaxAgeMs: options.requiredMatrixMaxAgeMs,
    staleMatrixReports,
  }
}

function generatedEvidenceKindDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedEvidenceKinds)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedEvidenceKinds ?? {}))
  return {
    requiredGeneratedEvidenceKindRequirements: required,
    missingRequiredGeneratedEvidenceKinds: required.filter((kind) => !available.has(kind)),
  }
}

function generatedEvidenceKindRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedEvidenceKindRequirements ?? []
  const missing = aggregate.missingRequiredGeneratedEvidenceKinds ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedEvidenceKindRequirements: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingRequiredGeneratedEvidenceKinds: missing.join(",") } : {}),
  }
}

function generatedMatrixLimitationDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedMatrixLimitations)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedMatrixLimitations ?? {}))
  return {
    requiredGeneratedMatrixLimitationRequirements: required,
    missingRequiredGeneratedMatrixLimitations: required.filter((limitation) => !available.has(limitation)),
  }
}

function generatedMatrixLimitationRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedMatrixLimitationRequirements ?? []
  const missing = aggregate.missingRequiredGeneratedMatrixLimitations ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedMatrixLimitationRequirements: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingRequiredGeneratedMatrixLimitations: missing.join(",") } : {}),
  }
}

function generatedValidationSuiteFailureRootDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedValidationSuiteFailureRoots)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedValidationSuiteFailureRoots ?? {}))
  return {
    requiredGeneratedValidationSuiteFailureRoots: required,
    missingGeneratedValidationSuiteFailureRoots: required.filter((root) => !available.has(root)),
  }
}

function generatedValidationSuiteFailureRootRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedValidationSuiteFailureRoots ?? []
  const missing = aggregate.missingGeneratedValidationSuiteFailureRoots ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedValidationSuiteFailureRoots: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedValidationSuiteFailureRoots: missing.join(",") } : {}),
  }
}

function generatedMatrixArtifactIndexDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedMatrixArtifactIndexes)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedMatrixArtifactIndexes ?? {}))
  return {
    requiredGeneratedMatrixArtifactIndexPaths: required,
    missingGeneratedMatrixArtifactIndexPaths: required.filter((indexPath) => !available.has(indexPath)),
  }
}

function generatedMatrixArtifactIndexRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedMatrixArtifactIndexPaths ?? []
  const missing = aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedMatrixArtifactIndexes: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedMatrixArtifactIndexes: missing.join(",") } : {}),
  }
}

function providerAccountAliasDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredProviderAccountAliases)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.providerAccountAliases ?? {}))
  return {
    requiredProviderAccountAliases: required,
    missingProviderAccountAliases: required.filter((alias) => !available.has(alias)),
  }
}

function providerAccountAliasRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredProviderAccountAliases ?? []
  const missing = aggregate.missingProviderAccountAliases ?? []
  return {
    ...(required.length > 0 ? { requiredProviderAccountAliases: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingProviderAccountAliases: missing.join(",") } : {}),
  }
}

function formatAggregateSummaryWithFreshness(aggregate) {
  const lines = [formatDrillArtifactIndexAggregateSummary(aggregate)]
  if (aggregate.requiredArtifactMaxAgeMs !== undefined) {
    lines.push(`artifact_required_max_age_ms=${aggregate.requiredArtifactMaxAgeMs} stale_indexes=${(aggregate.staleArtifactIndexes ?? []).length}`)
    for (const staleIndex of aggregate.staleArtifactIndexes ?? []) {
      lines.push(`- stale_artifact_index=${staleIndex.source ?? "unknown"} created_at=${staleIndex.createdAt} age_ms=${staleIndex.ageMs} max_age_ms=${staleIndex.maxAgeMs}`)
    }
  }
  if (aggregate.requiredMatrixMaxAgeMs !== undefined) {
    lines.push(`matrix_required_max_age_ms=${aggregate.requiredMatrixMaxAgeMs} stale_reports=${(aggregate.staleMatrixReports ?? []).length}`)
    for (const staleReport of aggregate.staleMatrixReports ?? []) {
      lines.push(`- stale_matrix_report=${staleReport.source ?? "unknown"} matrix=${staleReport.matrix} completed_at=${staleReport.completedAt} age_ms=${staleReport.ageMs} max_age_ms=${staleReport.maxAgeMs}`)
    }
  }
  if (aggregate.requiredGeneratedEvidenceKindRequirements !== undefined) {
    const missing = aggregate.missingRequiredGeneratedEvidenceKinds ?? []
    lines.push(`generated_evidence_kinds_required=${aggregate.requiredGeneratedEvidenceKindRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedMatrixLimitationRequirements !== undefined) {
    const missing = aggregate.missingRequiredGeneratedMatrixLimitations ?? []
    lines.push(`generated_matrix_limitations_required=${aggregate.requiredGeneratedMatrixLimitationRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedValidationSuiteFailureRoots !== undefined) {
    const missing = aggregate.missingGeneratedValidationSuiteFailureRoots ?? []
    lines.push(`generated_validation_suite_failure_roots_required=${aggregate.requiredGeneratedValidationSuiteFailureRoots.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedMatrixArtifactIndexPaths !== undefined) {
    const missing = aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []
    lines.push(`generated_matrix_artifact_indexes_required=${aggregate.requiredGeneratedMatrixArtifactIndexPaths.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredProviderAccountAliases !== undefined) {
    const missing = aggregate.missingProviderAccountAliases ?? []
    lines.push(`provider_account_aliases_required=${aggregate.requiredProviderAccountAliases.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if ((aggregate.staleArtifactIndexes ?? []).length > 0) {
    lines.push("next: regenerate stale drill artifact indexes before using them as validation evidence")
  }
  if ((aggregate.staleMatrixReports ?? []).length > 0) {
    lines.push("next: regenerate stale drill matrix reports before using them as validation evidence")
  }
  if ((aggregate.missingRequiredGeneratedEvidenceKinds ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record generated evidence kinds: ${aggregate.missingRequiredGeneratedEvidenceKinds.join(", ")}`)
  }
  if ((aggregate.missingRequiredGeneratedMatrixLimitations ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record generated matrix limitations: ${aggregate.missingRequiredGeneratedMatrixLimitations.join(", ")}`)
  }
  if ((aggregate.missingGeneratedValidationSuiteFailureRoots ?? []).length > 0) {
    lines.push(`next: rerun generated validation suites with --preserve-failure-root or include the artifact index that records the preserved failure root: ${aggregate.missingGeneratedValidationSuiteFailureRoots.join(", ")}`)
  }
  if ((aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []).length > 0) {
    lines.push(`next: rerun generated matrix drills with artifact indexes or include the artifact index that records generated matrix artifact indexes: ${aggregate.missingGeneratedMatrixArtifactIndexPaths.join(", ")}`)
  }
  if ((aggregate.missingProviderAccountAliases ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record provider account aliases: ${aggregate.missingProviderAccountAliases.join(", ")}`)
  }
  return lines.join("\n")
}

main().catch((error) => {
  console.error(`[drill-artifact-index-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})

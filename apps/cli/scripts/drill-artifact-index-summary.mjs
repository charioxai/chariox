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
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  formatDrillAggregateNextActionSourceDetails,
} from "./lib/drill-aggregate-actions.mjs"
import {
  parseProviderAccountAlias,
  validateDrillProvider,
} from "./lib/drill-provider-profiles.mjs"
import { redactDrillSecretText } from "./lib/drill-secrets.mjs"
import { isKnownDrillArtifactEvidenceRepo } from "./lib/drill-evidence-repos.mjs"
import { validateDrillGeneratedEvidenceKind } from "./lib/drill-generated-evidence-metadata.mjs"
import { validateDrillGeneratedMatrixName } from "./lib/drill-generated-matrix-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./lib/drill-generated-matrix-limitations.mjs"
import {
  drillFailureOwnerForClassification,
  validateDrillFailureClassification,
} from "./lib/drill-failure-taxonomy.mjs"
import {
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalNextAction,
  validateDrillRuntimeSignal,
  validateDrillRuntimeSignalOwner,
} from "./lib/drill-runtime-signals.mjs"
import { validateDrillArtifactValidationPreset } from "./lib/drill-validation-gate-presets.mjs"

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
    "  --require-generated-matrix-name NAME",
    "                         Exit non-zero when generated matrix name metadata is missing; repeatable",
    "  --require-generated-matrix-repo REPO",
    "                         Exit non-zero when generated matrix repo metadata is missing; repeatable",
    "  --require-generated-validation-suite-failure-root PATH",
    "                         Exit non-zero when generated validation-suite failure-root metadata is missing; repeatable",
    "  --require-generated-validation-suite-artifact-index PATH",
    "                         Exit non-zero when generated validation-suite artifact-index metadata is missing; repeatable",
    "  --require-generated-matrix-artifact-index PATH",
    "                         Exit non-zero when generated matrix artifact-index metadata is missing; repeatable",
    "  --require-provider-account-alias P=A",
    "                         Exit non-zero when provider account alias metadata is missing; repeatable",
    "  --require-planned-owner OWNER",
    "                         Exit non-zero when dry-run planned owner metadata is missing; repeatable",
    "  --require-planned-classification CLASS",
    "                         Exit non-zero when dry-run planned classification metadata is missing; repeatable",
    "  --require-validation-preset PRESET",
    "                         Exit non-zero when validation preset metadata is missing; repeatable",
    "  --require-failure-classification CLASS",
    "                         Exit non-zero when required failure classification metadata is missing; repeatable",
    "  --require-runtime-signal ID[,ID]",
    "                         Exit non-zero when runtime signal metadata is missing; repeatable",
    "  --require-runtime-signal-owner OWNER[,OWNER]",
    "                         Exit non-zero when runtime signal owner metadata is missing; repeatable",
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
  Object.assign(aggregate, generatedMatrixNameDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedMatrixRepoDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedValidationSuiteArtifactIndexDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedValidationSuiteFailureRootDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, generatedMatrixArtifactIndexDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, providerAccountAliasDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, plannedDiagnosticRequirementDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, validationPresetDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, failureClassificationDiagnosticsFor(aggregate, options))
  Object.assign(aggregate, runtimeSignalDiagnosticsFor(aggregate, options))
  aggregate.nextActions = artifactIndexSummaryNextActions(aggregate)
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
        ...generatedMatrixNameRequirementMetadataFor(aggregate),
        ...generatedMatrixRepoRequirementMetadataFor(aggregate),
        ...generatedValidationSuiteArtifactIndexRequirementMetadataFor(aggregate),
        ...generatedValidationSuiteFailureRootRequirementMetadataFor(aggregate),
        ...generatedMatrixArtifactIndexRequirementMetadataFor(aggregate),
        ...providerAccountAliasRequirementMetadataFor(aggregate),
        ...plannedDiagnosticRequirementMetadataFor(aggregate),
        ...validationPresetRequirementMetadataFor(aggregate),
        ...failureClassificationRequirementMetadataFor(aggregate),
        ...runtimeSignalRequirementMetadataFor(aggregate),
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
    || (aggregate.missingRequiredGeneratedMatrixNames ?? []).length > 0
    || (aggregate.missingRequiredGeneratedMatrixRepos ?? []).length > 0
    || (aggregate.missingGeneratedValidationSuiteArtifactIndexPaths ?? []).length > 0
    || (aggregate.missingGeneratedValidationSuiteFailureRootRequirements ?? []).length > 0
    || (aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []).length > 0
    || (aggregate.missingProviderAccountAliases ?? []).length > 0
    || (aggregate.missingPlannedOwners ?? []).length > 0
    || (aggregate.missingPlannedClassifications ?? []).length > 0
    || (aggregate.missingValidationPresets ?? []).length > 0
    || (aggregate.missingFailureClassificationRequirements ?? []).length > 0
    || (aggregate.missingRuntimeSignalRequirements ?? []).length > 0
    || (aggregate.missingRuntimeSignalOwnerRequirements ?? []).length > 0
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
    requiredGeneratedMatrixNames: [],
    requiredGeneratedMatrixRepos: [],
    requiredGeneratedValidationSuiteArtifactIndexes: [],
    requiredGeneratedValidationSuiteFailureRoots: [],
    requiredGeneratedMatrixArtifactIndexes: [],
    requiredFailureClassifications: [],
    requiredMatrixMaxAgeMs: null,
    requiredPlannedClassifications: [],
    requiredPlannedOwners: [],
    requiredProviderAccountAliases: [],
    requiredRuntimeSignalOwners: [],
    requiredRuntimeSignals: [],
    requiredValidationPresets: [],
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
    } else if (arg === "--require-generated-matrix-name") {
      options.requiredGeneratedMatrixNames.push(parseGeneratedMatrixNameRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-matrix-name=")) {
      options.requiredGeneratedMatrixNames.push(parseGeneratedMatrixNameRequirement(
        arg.slice("--require-generated-matrix-name=".length),
        "--require-generated-matrix-name",
      ))
    } else if (arg === "--require-generated-matrix-repo") {
      options.requiredGeneratedMatrixRepos.push(parseGeneratedMatrixRepoRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-matrix-repo=")) {
      options.requiredGeneratedMatrixRepos.push(parseGeneratedMatrixRepoRequirement(
        arg.slice("--require-generated-matrix-repo=".length),
        "--require-generated-matrix-repo",
      ))
    } else if (arg === "--require-generated-validation-suite-failure-root") {
      options.requiredGeneratedValidationSuiteFailureRoots.push(parseDiagnosticRequirementText(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-validation-suite-failure-root=")) {
      options.requiredGeneratedValidationSuiteFailureRoots.push(parseDiagnosticRequirementText(
        arg.slice("--require-generated-validation-suite-failure-root=".length),
        "--require-generated-validation-suite-failure-root",
      ))
    } else if (arg === "--require-generated-validation-suite-artifact-index") {
      options.requiredGeneratedValidationSuiteArtifactIndexes.push(parseDiagnosticRequirementText(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-validation-suite-artifact-index=")) {
      options.requiredGeneratedValidationSuiteArtifactIndexes.push(parseDiagnosticRequirementText(
        arg.slice("--require-generated-validation-suite-artifact-index=".length),
        "--require-generated-validation-suite-artifact-index",
      ))
    } else if (arg === "--require-generated-matrix-artifact-index") {
      options.requiredGeneratedMatrixArtifactIndexes.push(parseDiagnosticRequirementText(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-generated-matrix-artifact-index=")) {
      options.requiredGeneratedMatrixArtifactIndexes.push(parseDiagnosticRequirementText(
        arg.slice("--require-generated-matrix-artifact-index=".length),
        "--require-generated-matrix-artifact-index",
      ))
    } else if (arg === "--require-provider-account-alias") {
      options.requiredProviderAccountAliases.push(parseProviderAccountAliasRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-provider-account-alias=")) {
      options.requiredProviderAccountAliases.push(parseProviderAccountAliasRequirement(
        arg.slice("--require-provider-account-alias=".length),
        "--require-provider-account-alias",
      ))
    } else if (arg === "--require-planned-owner") {
      options.requiredPlannedOwners.push(parseDiagnosticRequirementText(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-planned-owner=")) {
      options.requiredPlannedOwners.push(parseDiagnosticRequirementText(
        arg.slice("--require-planned-owner=".length),
        "--require-planned-owner",
      ))
    } else if (arg === "--require-planned-classification") {
      options.requiredPlannedClassifications.push(parseDiagnosticRequirementText(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-planned-classification=")) {
      options.requiredPlannedClassifications.push(parseDiagnosticRequirementText(
        arg.slice("--require-planned-classification=".length),
        "--require-planned-classification",
      ))
    } else if (arg === "--require-validation-preset") {
      options.requiredValidationPresets.push(parseValidationPresetRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-validation-preset=")) {
      options.requiredValidationPresets.push(parseValidationPresetRequirement(
        arg.slice("--require-validation-preset=".length),
        "--require-validation-preset",
      ))
    } else if (arg === "--require-failure-classification") {
      options.requiredFailureClassifications.push(parseFailureClassificationRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-failure-classification=")) {
      options.requiredFailureClassifications.push(parseFailureClassificationRequirement(
        arg.slice("--require-failure-classification=".length),
        "--require-failure-classification",
      ))
    } else if (arg === "--require-runtime-signal") {
      options.requiredRuntimeSignals.push(...parseRuntimeSignalRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-runtime-signal=")) {
      options.requiredRuntimeSignals.push(...parseRuntimeSignalRequirement(
        arg.slice("--require-runtime-signal=".length),
        "--require-runtime-signal",
      ))
    } else if (arg === "--require-runtime-signal-owner") {
      options.requiredRuntimeSignalOwners.push(...parseRuntimeSignalOwnerRequirement(readValue(argv, index, arg), arg))
      index += 1
    } else if (arg.startsWith("--require-runtime-signal-owner=")) {
      options.requiredRuntimeSignalOwners.push(...parseRuntimeSignalOwnerRequirement(
        arg.slice("--require-runtime-signal-owner=".length),
        "--require-runtime-signal-owner",
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
    validateDrillProvider(provider, "provider account alias provider", {
      message: () => `unknown provider account alias provider: ${provider}`,
    })
    return `${provider}=${alias}`
  } catch (error) {
    throw new Error(`${flag} has invalid value: ${error.message}`)
  }
}

function parseGeneratedEvidenceKindRequirement(value, flag) {
  validateDrillGeneratedEvidenceKind(value, flag, {
    message: () => `${flag} has unknown generated evidence kind: ${value}`,
  })
  return value
}

function parseGeneratedMatrixLimitationRequirement(value, flag) {
  validateDrillGeneratedMatrixLimitation(value, flag, {
    message: () => `${flag} has unknown generated matrix limitation: ${value}`,
  })
  return value
}

function parseGeneratedMatrixNameRequirement(value, flag) {
  const matrixName = parseDiagnosticRequirementText(value, flag)
  validateDrillGeneratedMatrixName(matrixName, {
    secretSource: flag,
    unknownSource: flag,
    message: () => `${flag} has unknown generated matrix name: ${matrixName}`,
  })
  return matrixName
}

function parseGeneratedMatrixRepoRequirement(value, flag) {
  if (!isKnownDrillArtifactEvidenceRepo(value)) {
    throw new Error(`${flag} has unknown generated matrix repo: ${value}`)
  }
  return value
}

function parseValidationPresetRequirement(value, flag) {
  validateDrillArtifactValidationPreset(value, flag, {
    message: () => `${flag} has unknown validation preset: ${value}`,
  })
  return value
}

function parseFailureClassificationRequirement(value, flag) {
  validateDrillFailureClassification(value, flag, {
    label: "failure classification",
  })
  return value
}

function parseRuntimeSignalRequirement(value, flag) {
  const signals = String(value ?? "").split(",").map((signal) => signal.trim()).filter(Boolean)
  if (signals.length === 0) {
    throw new Error(`${flag} requires a value`)
  }
  for (const signal of signals) {
    validateDrillRuntimeSignal(signal, flag, {
      message: () => `${flag} has unknown runtime signal: ${signal}`,
    })
  }
  return signals
}

function parseRuntimeSignalOwnerRequirement(value, flag) {
  const owners = String(value ?? "").split(",").map((owner) => owner.trim()).filter(Boolean)
  if (owners.length === 0) {
    throw new Error(`${flag} requires a value`)
  }
  for (const owner of owners) {
    validateDrillRuntimeSignalOwner(owner, flag, {
      message: () => `${flag} has unknown runtime signal owner: ${owner}`,
    })
  }
  return owners
}

function parseDiagnosticRequirementText(value, flag) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${flag} requires a value`)
  }
  if (redactDrillSecretText(value) !== value) {
    throw new Error(`${flag} includes secret-looking diagnostic text`)
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
    ...(required.length > 0 ? { requiredGeneratedMatrixLimitations: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedMatrixLimitations: missing.join(",") } : {}),
  }
}

function generatedMatrixNameDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedMatrixNames)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedMatrixNames ?? {}))
  return {
    requiredGeneratedMatrixNameRequirements: required,
    missingRequiredGeneratedMatrixNames: required.filter((name) => !available.has(name)),
  }
}

function generatedMatrixNameRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedMatrixNameRequirements ?? []
  const missing = aggregate.missingRequiredGeneratedMatrixNames ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedMatrixNames: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedMatrixNames: missing.join(",") } : {}),
  }
}

function generatedMatrixRepoDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedMatrixRepos)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedMatrixRepos ?? {}))
  return {
    requiredGeneratedMatrixRepoRequirements: required,
    missingRequiredGeneratedMatrixRepos: required.filter((repo) => !available.has(repo)),
  }
}

function generatedMatrixRepoRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedMatrixRepoRequirements ?? []
  const missing = aggregate.missingRequiredGeneratedMatrixRepos ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedMatrixRepos: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedMatrixRepos: missing.join(",") } : {}),
  }
}

function generatedValidationSuiteFailureRootDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedValidationSuiteFailureRoots)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedValidationSuiteFailureRoots ?? {}))
  return {
    requiredGeneratedValidationSuiteFailureRootRequirements: required,
    missingGeneratedValidationSuiteFailureRootRequirements: required.filter((root) => !available.has(root)),
  }
}

function generatedValidationSuiteFailureRootRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedValidationSuiteFailureRootRequirements ?? []
  const missing = aggregate.missingGeneratedValidationSuiteFailureRootRequirements ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedValidationSuiteFailureRoots: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedValidationSuiteFailureRoots: missing.join(",") } : {}),
  }
}

function generatedValidationSuiteArtifactIndexDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredGeneratedValidationSuiteArtifactIndexes)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.generatedValidationSuiteArtifactIndexes ?? {}))
  return {
    requiredGeneratedValidationSuiteArtifactIndexPaths: required,
    missingGeneratedValidationSuiteArtifactIndexPaths: required.filter((indexPath) => !available.has(indexPath)),
  }
}

function generatedValidationSuiteArtifactIndexRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredGeneratedValidationSuiteArtifactIndexPaths ?? []
  const missing = aggregate.missingGeneratedValidationSuiteArtifactIndexPaths ?? []
  return {
    ...(required.length > 0 ? { requiredGeneratedValidationSuiteArtifactIndexes: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingGeneratedValidationSuiteArtifactIndexes: missing.join(",") } : {}),
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

function plannedDiagnosticRequirementDiagnosticsFor(aggregate, options) {
  const requiredOwners = [...new Set(options.requiredPlannedOwners)].sort()
  const requiredClassifications = [...new Set(options.requiredPlannedClassifications)].sort()
  if (requiredOwners.length === 0 && requiredClassifications.length === 0) return {}
  const availableOwners = new Set(Object.keys(aggregate.plannedOwners ?? {}))
  const availableClassifications = new Set(Object.keys(aggregate.plannedClassifications ?? {}))
  return {
    requiredPlannedOwners: requiredOwners,
    missingPlannedOwners: requiredOwners.filter((owner) => !availableOwners.has(owner)),
    requiredPlannedClassifications: requiredClassifications,
    missingPlannedClassifications: requiredClassifications.filter((classification) => !availableClassifications.has(classification)),
  }
}

function plannedDiagnosticRequirementMetadataFor(aggregate) {
  const requiredOwners = aggregate.requiredPlannedOwners ?? []
  const missingOwners = aggregate.missingPlannedOwners ?? []
  const requiredClassifications = aggregate.requiredPlannedClassifications ?? []
  const missingClassifications = aggregate.missingPlannedClassifications ?? []
  return {
    ...(requiredOwners.length > 0 ? { requiredPlannedOwners: requiredOwners.join(",") } : {}),
    ...(missingOwners.length > 0 ? { missingPlannedOwners: missingOwners.join(",") } : {}),
    ...(requiredClassifications.length > 0 ? { requiredPlannedClassifications: requiredClassifications.join(",") } : {}),
    ...(missingClassifications.length > 0 ? { missingPlannedClassifications: missingClassifications.join(",") } : {}),
  }
}

function validationPresetDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredValidationPresets)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.validationPresets ?? {}))
  return {
    requiredValidationPresets: required,
    missingValidationPresets: required.filter((preset) => !available.has(preset)),
  }
}

function validationPresetRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredValidationPresets ?? []
  const missing = aggregate.missingValidationPresets ?? []
  return {
    ...(required.length > 0 ? { requiredValidationPresets: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingValidationPresets: missing.join(",") } : {}),
  }
}

function failureClassificationDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredFailureClassifications)].sort()
  if (required.length === 0) return {}
  const available = new Set(Object.keys(aggregate.requiredFailureClassifications ?? {}))
  return {
    requiredFailureClassificationRequirements: required,
    missingFailureClassificationRequirements: required.filter((classification) => !available.has(classification)),
  }
}

function failureClassificationRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredFailureClassificationRequirements ?? []
  const missing = aggregate.missingFailureClassificationRequirements ?? []
  return {
    ...(required.length > 0 ? { requiredFailureClassifications: required.join(",") } : {}),
    ...(missing.length > 0 ? { missingFailureClassifications: missing.join(",") } : {}),
  }
}

function runtimeSignalDiagnosticsFor(aggregate, options) {
  const required = [...new Set(options.requiredRuntimeSignals)].sort()
  const requiredOwners = [...new Set(options.requiredRuntimeSignalOwners)].sort()
  if (required.length === 0 && requiredOwners.length === 0) return {}
  const available = new Set(Object.keys(aggregate.runtimeSignals ?? {}))
  const availableOwners = new Set(Object.keys(aggregate.runtimeSignalOwners ?? {}))
  return {
    requiredRuntimeSignalRequirements: required,
    missingRuntimeSignalRequirements: required.filter((signal) => !available.has(signal)),
    requiredRuntimeSignalOwnerRequirements: requiredOwners,
    missingRuntimeSignalOwnerRequirements: requiredOwners.filter((owner) => !availableOwners.has(owner)),
  }
}

function runtimeSignalRequirementMetadataFor(aggregate) {
  const required = aggregate.requiredRuntimeSignalRequirements ?? []
  const missing = aggregate.missingRuntimeSignalRequirements ?? []
  const requiredOwners = [...new Set([
    ...drillRuntimeSignalOwnersFor(required),
    ...(aggregate.requiredRuntimeSignalOwnerRequirements ?? []),
  ])].sort()
  const missingOwners = [...new Set([
    ...drillRuntimeSignalOwnersFor(missing),
    ...(aggregate.missingRuntimeSignalOwnerRequirements ?? []),
  ])].sort()
  return {
    ...(required.length > 0 ? { requiredRuntimeSignals: required.join(",") } : {}),
    ...(requiredOwners.length > 0 ? { requiredRuntimeSignalOwners: requiredOwners.join(",") } : {}),
    ...(missing.length > 0 ? { missingRuntimeSignals: missing.join(",") } : {}),
    ...(missingOwners.length > 0 ? { missingRuntimeSignalOwners: missingOwners.join(",") } : {}),
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
  if (aggregate.requiredGeneratedMatrixNameRequirements !== undefined) {
    const missing = aggregate.missingRequiredGeneratedMatrixNames ?? []
    lines.push(`generated_matrix_names_required=${aggregate.requiredGeneratedMatrixNameRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedMatrixRepoRequirements !== undefined) {
    const missing = aggregate.missingRequiredGeneratedMatrixRepos ?? []
    lines.push(`generated_matrix_repos_required=${aggregate.requiredGeneratedMatrixRepoRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedValidationSuiteFailureRootRequirements !== undefined) {
    const missing = aggregate.missingGeneratedValidationSuiteFailureRootRequirements ?? []
    lines.push(`generated_validation_suite_failure_roots_required=${aggregate.requiredGeneratedValidationSuiteFailureRootRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedValidationSuiteArtifactIndexPaths !== undefined) {
    const missing = aggregate.missingGeneratedValidationSuiteArtifactIndexPaths ?? []
    lines.push(`generated_validation_suite_artifact_indexes_required=${aggregate.requiredGeneratedValidationSuiteArtifactIndexPaths.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredGeneratedMatrixArtifactIndexPaths !== undefined) {
    const missing = aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []
    lines.push(`generated_matrix_artifact_indexes_required=${aggregate.requiredGeneratedMatrixArtifactIndexPaths.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredProviderAccountAliases !== undefined) {
    const missing = aggregate.missingProviderAccountAliases ?? []
    lines.push(`provider_account_aliases_required=${aggregate.requiredProviderAccountAliases.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredPlannedOwners !== undefined) {
    const missing = aggregate.missingPlannedOwners ?? []
    lines.push(`planned_owners_required=${aggregate.requiredPlannedOwners.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredPlannedClassifications !== undefined) {
    const missing = aggregate.missingPlannedClassifications ?? []
    lines.push(`planned_classifications_required=${aggregate.requiredPlannedClassifications.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredValidationPresets !== undefined) {
    const missing = aggregate.missingValidationPresets ?? []
    lines.push(`validation_presets_required=${aggregate.requiredValidationPresets.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredFailureClassificationRequirements !== undefined) {
    const missing = aggregate.missingFailureClassificationRequirements ?? []
    lines.push(`failure_classifications_required=${aggregate.requiredFailureClassificationRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredRuntimeSignalRequirements !== undefined) {
    const missing = aggregate.missingRuntimeSignalRequirements ?? []
    lines.push(`runtime_signals_required=${aggregate.requiredRuntimeSignalRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if (aggregate.requiredRuntimeSignalOwnerRequirements !== undefined) {
    const missing = aggregate.missingRuntimeSignalOwnerRequirements ?? []
    lines.push(`runtime_signal_owners_required=${aggregate.requiredRuntimeSignalOwnerRequirements.join(",") || "none"} missing=${missing.join(",") || "none"}`)
  }
  if ((aggregate.nextActions ?? []).length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count} next=${action.nextAction}`)
      const sources = formatDrillAggregateNextActionSourceDetails(action.sourceDetails)
      if (sources) {
        lines.push(`  sources: ${sources}`)
      }
    }
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
  if ((aggregate.missingRequiredGeneratedMatrixNames ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record generated matrix names: ${aggregate.missingRequiredGeneratedMatrixNames.join(", ")}`)
  }
  if ((aggregate.missingRequiredGeneratedMatrixRepos ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record generated matrix repos: ${aggregate.missingRequiredGeneratedMatrixRepos.join(", ")}`)
  }
  if ((aggregate.missingGeneratedValidationSuiteFailureRootRequirements ?? []).length > 0) {
    lines.push(`next: rerun generated validation suites with --preserve-failure-root or include the artifact index that records the preserved failure root: ${aggregate.missingGeneratedValidationSuiteFailureRootRequirements.join(", ")}`)
  }
  if ((aggregate.missingGeneratedValidationSuiteArtifactIndexPaths ?? []).length > 0) {
    lines.push(`next: rerun generated validation suites with artifact indexes or include the artifact index that records generated validation-suite artifact indexes: ${aggregate.missingGeneratedValidationSuiteArtifactIndexPaths.join(", ")}`)
  }
  if ((aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []).length > 0) {
    lines.push(`next: rerun generated matrix drills with artifact indexes or include the artifact index that records generated matrix artifact indexes: ${aggregate.missingGeneratedMatrixArtifactIndexPaths.join(", ")}`)
  }
  if ((aggregate.missingProviderAccountAliases ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record provider account aliases: ${aggregate.missingProviderAccountAliases.join(", ")}`)
  }
  if ((aggregate.missingPlannedOwners ?? []).length > 0) {
    lines.push(`next: include dry-run drill matrix artifact indexes with planned owner coverage: ${aggregate.missingPlannedOwners.join(", ")}`)
  }
  if ((aggregate.missingPlannedClassifications ?? []).length > 0) {
    lines.push(`next: include dry-run drill matrix artifact indexes with planned classification coverage: ${aggregate.missingPlannedClassifications.join(", ")}`)
  }
  if ((aggregate.missingValidationPresets ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes that record validation presets: ${aggregate.missingValidationPresets.join(", ")}`)
  }
  if ((aggregate.missingFailureClassificationRequirements ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes with required failure classification coverage: ${aggregate.missingFailureClassificationRequirements.join(", ")}`)
  }
  for (const signal of aggregate.missingRuntimeSignalRequirements ?? []) {
    lines.push(`next: ${drillRuntimeSignalNextAction(signal, { target: "artifact-index" })}`)
  }
  if ((aggregate.missingRuntimeSignalOwnerRequirements ?? []).length > 0) {
    lines.push(`next: include drill artifact indexes with runtime signal owner coverage: ${aggregate.missingRuntimeSignalOwnerRequirements.join(", ")}`)
  }
  return lines.join("\n")
}

function artifactIndexSummaryNextActions(aggregate) {
  const nextActions = new Map()
  if ((aggregate.staleArtifactIndexes ?? []).length > 0) {
    countArtifactIndexSummaryNextAction(nextActions, {
      classification: "artifact-staleness",
      nextAction: "regenerate stale drill artifact indexes before using them as validation evidence",
      count: aggregate.staleArtifactIndexes.length,
      sourceDetails: aggregate.staleArtifactIndexes.map(staleArtifactIndexSourceDetail),
    })
  }
  if ((aggregate.staleMatrixReports ?? []).length > 0) {
    countArtifactIndexSummaryNextAction(nextActions, {
      classification: "matrix-staleness",
      nextAction: "regenerate stale drill matrix reports before using them as validation evidence",
      count: aggregate.staleMatrixReports.length,
      sourceDetails: aggregate.staleMatrixReports.map(staleMatrixReportSourceDetail),
    })
  }
  addMissingListActions(
    nextActions,
    aggregate.missingRequiredGeneratedEvidenceKinds,
    "generated-evidence",
    "include drill artifact indexes that record generated evidence kinds",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingRequiredGeneratedMatrixLimitations,
    "generated-evidence",
    "include drill artifact indexes that record generated matrix limitations",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingRequiredGeneratedMatrixNames,
    "generated-evidence",
    "include drill artifact indexes that record generated matrix names",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingRequiredGeneratedMatrixRepos,
    "generated-evidence",
    "include drill artifact indexes that record generated matrix repos",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingGeneratedValidationSuiteFailureRootRequirements,
    "generated-evidence",
    "rerun generated validation suites with --preserve-failure-root or include the artifact index that records the preserved failure root",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingGeneratedValidationSuiteArtifactIndexPaths,
    "generated-evidence",
    "rerun generated validation suites with artifact indexes or include the artifact index that records generated validation-suite artifact indexes",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingGeneratedMatrixArtifactIndexPaths,
    "generated-evidence",
    "rerun generated matrix drills with artifact indexes or include the artifact index that records generated matrix artifact indexes",
  )
  addMissingListActions(
    nextActions,
    aggregate.missingProviderAccountAliases,
    "provider-account",
    "include drill artifact indexes that record provider account aliases",
    "provider-account",
  )
  for (const owner of aggregate.missingPlannedOwners ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner,
      classification: "artifact-coverage",
      nextAction: `include dry-run drill matrix artifact indexes with planned owner coverage: ${owner}`,
    })
  }
  for (const classification of aggregate.missingPlannedClassifications ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner: artifactIndexSummaryOwnerForClassification(classification),
      classification,
      nextAction: `include dry-run drill matrix artifact indexes with planned classification coverage: ${classification}`,
    })
  }
  addMissingListActions(
    nextActions,
    aggregate.missingValidationPresets,
    "artifact-coverage",
    "include drill artifact indexes that record validation presets",
  )
  for (const classification of aggregate.missingFailureClassificationRequirements ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner: artifactIndexSummaryOwnerForClassification(classification),
      classification,
      nextAction: `include drill artifact indexes with required failure classification coverage: ${classification}`,
    })
  }
  for (const signal of aggregate.missingRuntimeSignalRequirements ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner: drillRuntimeSignalOwnersFor([signal])[0],
      classification: "runtime-signal-coverage",
      nextAction: drillRuntimeSignalNextAction(signal, { target: "artifact-index" }),
    })
  }
  for (const owner of aggregate.missingRuntimeSignalOwnerRequirements ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner,
      classification: "runtime-signal-coverage",
      nextAction: `include drill artifact indexes with runtime signal owner coverage: ${owner}`,
    })
  }
  return formatDrillAggregateNextActionCounts(nextActions)
}

function addMissingListActions(nextActions, values, classification, prefix, owner = "validation-harness") {
  if ((values ?? []).length === 0) return
  countArtifactIndexSummaryNextAction(nextActions, {
    owner,
    classification,
    nextAction: `${prefix}: ${values.join(", ")}`,
  })
}

function countArtifactIndexSummaryNextAction(nextActions, {
  owner = "validation-harness",
  classification,
  nextAction,
  count = 1,
  sourceDetails,
}) {
  countDrillAggregateNextAction(nextActions, { owner, classification, nextAction, count, sourceDetails })
}

function staleArtifactIndexSourceDetail(staleIndex) {
  return {
    source: "artifact-index",
    ...(staleIndex.source ? { reportPath: staleIndex.source } : {}),
  }
}

function staleMatrixReportSourceDetail(staleReport) {
  return {
    source: staleReport.matrix ?? "matrix-report",
    ...(staleReport.matrix ? { matrix: staleReport.matrix } : {}),
    ...(staleReport.source ? { reportPath: staleReport.source } : {}),
  }
}

function artifactIndexSummaryOwnerForClassification(classification) {
  return drillFailureOwnerForClassification(classification, { fallback: "validation-harness" })
}

main().catch((error) => {
  console.error(`[drill-artifact-index-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})

import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
} from "./drill-aggregate-actions.mjs"
import { drillFailureOwnerForClassification } from "./drill-failure-taxonomy.mjs"
import {
  drillRuntimeSignalNextAction,
  drillRuntimeSignalOwnersFor,
} from "./drill-runtime-signals.mjs"
import { drillRuntimeAuthorityInvariantOwner } from "./drill-runtime-authority-invariants.mjs"

export function artifactIndexSummaryNextActions(aggregate) {
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
    "validation-harness",
    { sourceDetails: (aggregate.missingGeneratedValidationSuiteFailureRootRequirements ?? []).map(missingPathSourceDetail) },
  )
  addMissingListActions(
    nextActions,
    aggregate.missingGeneratedValidationSuiteArtifactIndexPaths,
    "generated-evidence",
    "rerun generated validation suites with artifact indexes or include the artifact index that records generated validation-suite artifact indexes",
    "validation-harness",
    { sourceDetails: (aggregate.missingGeneratedValidationSuiteArtifactIndexPaths ?? []).map(missingPathSourceDetail) },
  )
  addMissingListActions(
    nextActions,
    aggregate.missingGeneratedMatrixArtifactIndexPaths,
    "generated-evidence",
    "rerun generated matrix drills with artifact indexes or include the artifact index that records generated matrix artifact indexes",
    "validation-harness",
    { sourceDetails: (aggregate.missingGeneratedMatrixArtifactIndexPaths ?? []).map(missingPathSourceDetail) },
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
  for (const invariant of aggregate.missingRuntimeAuthorityInvariantRequirements ?? []) {
    countArtifactIndexSummaryNextAction(nextActions, {
      owner: drillRuntimeAuthorityInvariantOwner(invariant),
      classification: "runtime-authority-coverage",
      nextAction: `include drill artifact indexes with runtime authority invariant coverage: ${invariant}`,
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

function addMissingListActions(nextActions, values, classification, prefix, owner = "validation-harness", { sourceDetails } = {}) {
  if ((values ?? []).length === 0) return
  countArtifactIndexSummaryNextAction(nextActions, {
    owner,
    classification,
    nextAction: `${prefix}: ${values.join(", ")}`,
    sourceDetails,
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

function missingPathSourceDetail(pathValue) {
  return { source: pathValue }
}

function artifactIndexSummaryOwnerForClassification(classification) {
  return drillFailureOwnerForClassification(classification, { fallback: "validation-harness" })
}

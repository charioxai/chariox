import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
} from "./drill-aggregate-actions.mjs"
import {
  drillFailureOwnerForClassification,
} from "./drill-failure-taxonomy.mjs"
import {
  drillRuntimeSignalNextAction,
  drillRuntimeSignalOwner,
} from "./drill-runtime-signals.mjs"

export function validationGateNextActions(checks) {
  const counts = new Map()
  if (checks.configuration.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "validation-gate",
      nextAction: "configure at least one platform bundle, artifact root, matrix root, or failure root before using the validation gate",
    })
  }
  if (checks.platformBundle.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
    })
    const missingCoverageAreas = checks.platformBundle.missingCoverageAreas ?? []
    if (missingCoverageAreas.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering: ${missingCoverageAreas.join(", ")}`,
      })
    }
    const missingRuntimeSignals = checks.platformBundle.missingRuntimeSignals ?? []
    if (missingRuntimeSignals.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering runtime signals: ${missingRuntimeSignals.join(", ")}`,
      })
      countMissingRuntimeSignalActions(counts, missingRuntimeSignals, { target: "platform-bundle" })
    }
    const missingRuntimeSignalOwners = checks.platformBundle.missingRuntimeSignalOwners ?? []
    if (missingRuntimeSignalOwners.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering runtime signal owners: ${missingRuntimeSignalOwners.join(", ")}`,
      })
      for (const owner of missingRuntimeSignalOwners) {
        countDrillAggregateNextAction(counts, {
          owner,
          classification: "runtime-signal-coverage",
          nextAction: `add runtime-signal contract coverage owned by ${owner} to the drill platform bundle`,
        })
      }
    }
    const missingFailureClassifications = checks.platformBundle.missingFailureClassifications ?? []
    if (missingFailureClassifications.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering failure classifications: ${missingFailureClassifications.join(", ")}`,
      })
    }
  }
  if (checks.artifacts.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "artifact-index",
      nextAction: "fix missing, unreadable, or tampered artifact indexes before using collected drill evidence",
    })
    const missingArtifactCoverageAreas = checks.artifacts.missingArtifactCoverageAreas ?? []
    if (missingArtifactCoverageAreas.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `run validation-suite artifacts covering: ${missingArtifactCoverageAreas.join(", ")}`,
      })
    }
    const missingArtifactSchemas = checks.artifacts.missingArtifactSchemas ?? []
    if (missingArtifactSchemas.length > 0) {
      const validationSuiteRunMissing = missingArtifactSchemas.includes("chariox.drill.validation_suite_run.v1")
      if (validationSuiteRunMissing) {
        countDrillAggregateNextAction(counts, {
          owner: "validation-harness",
          classification: "artifact-coverage",
          nextAction: "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate",
        })
      }
      const remainingSchemas = missingArtifactSchemas.filter((schema) => schema !== "chariox.drill.validation_suite_run.v1")
      if (remainingSchemas.length > 0) {
        countDrillAggregateNextAction(counts, {
          owner: "validation-harness",
          classification: "artifact-coverage",
          nextAction: `produce artifact evidence with schemas: ${remainingSchemas.join(", ")}`,
        })
      }
    }
    const missingArtifactKinds = checks.artifacts.missingArtifactKinds ?? []
    if (missingArtifactKinds.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `produce artifact evidence with kinds: ${missingArtifactKinds.join(", ")}`,
      })
    }
    const missingArtifactGeneratedEvidenceKinds = checks.artifacts.missingArtifactGeneratedEvidenceKinds ?? []
    if (missingArtifactGeneratedEvidenceKinds.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "generated-evidence",
        nextAction: `provide generated drill evidence with kinds: ${missingArtifactGeneratedEvidenceKinds.join(", ")}`,
      })
    }
    const missingArtifactGeneratedMatrixArtifactIndexes = checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes ?? []
    if (missingArtifactGeneratedMatrixArtifactIndexes.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "generated-evidence",
        nextAction: `provide artifact metadata with generated matrix artifact indexes: ${missingArtifactGeneratedMatrixArtifactIndexes.join(", ")}`,
        sourceDetails: missingArtifactGeneratedMatrixArtifactIndexes.map(missingPathSourceDetail),
      })
    }
    const missingArtifactGeneratedMatrixLimitations = checks.artifacts.missingArtifactGeneratedMatrixLimitations ?? []
    if (missingArtifactGeneratedMatrixLimitations.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "generated-evidence",
        nextAction: `provide artifact metadata with generated matrix limitations: ${missingArtifactGeneratedMatrixLimitations.join(", ")}`,
      })
    }
    const missingArtifactGeneratedValidationSuiteArtifactIndexes = checks.artifacts.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? []
    if (missingArtifactGeneratedValidationSuiteArtifactIndexes.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "generated-evidence",
        nextAction: `provide artifact metadata with generated validation-suite artifact indexes: ${missingArtifactGeneratedValidationSuiteArtifactIndexes.join(", ")}`,
        sourceDetails: missingArtifactGeneratedValidationSuiteArtifactIndexes.map(missingPathSourceDetail),
      })
    }
    const missingArtifactGeneratedValidationSuiteFailureRoots = checks.artifacts.missingArtifactGeneratedValidationSuiteFailureRoots ?? []
    if (missingArtifactGeneratedValidationSuiteFailureRoots.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "generated-evidence",
        nextAction: `provide artifact metadata with generated validation-suite failure roots: ${missingArtifactGeneratedValidationSuiteFailureRoots.join(", ")}`,
        sourceDetails: missingArtifactGeneratedValidationSuiteFailureRoots.map(missingPathSourceDetail),
      })
    }
    const missingArtifactEvidenceRepos = checks.artifacts.missingArtifactEvidenceRepos ?? []
    if (missingArtifactEvidenceRepos.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence from repos: ${missingArtifactEvidenceRepos.join(", ")}`,
      })
    }
    const missingArtifactProviderAccountAliases = checks.artifacts.missingArtifactProviderAccountAliases ?? []
    if (missingArtifactProviderAccountAliases.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "provider-account",
        classification: "provider-account",
        nextAction: `include non-secret provider account alias metadata in artifact indexes: ${missingArtifactProviderAccountAliases.join(", ")}`,
      })
    }
    const missingArtifactValidationPresets = checks.artifacts.missingArtifactValidationPresets ?? []
    if (missingArtifactValidationPresets.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence for validation presets: ${missingArtifactValidationPresets.join(", ")}`,
      })
    }
    const missingArtifactRuntimeAuthorityInvariants = checks.artifacts.missingArtifactRuntimeAuthorityInvariants ?? []
    if (missingArtifactRuntimeAuthorityInvariants.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence covering runtime authority invariants: ${missingArtifactRuntimeAuthorityInvariants.join(", ")}`,
      })
    }
    const missingArtifactRuntimeSignals = checks.artifacts.missingArtifactRuntimeSignals ?? []
    if (missingArtifactRuntimeSignals.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence covering runtime signals: ${missingArtifactRuntimeSignals.join(", ")}`,
      })
      countMissingRuntimeSignalActions(counts, missingArtifactRuntimeSignals, { target: "artifact-index" })
    }
    const missingArtifactRuntimeSignalOwners = checks.artifacts.missingArtifactRuntimeSignalOwners ?? []
    if (missingArtifactRuntimeSignalOwners.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence covering runtime signal owners: ${missingArtifactRuntimeSignalOwners.join(", ")}`,
      })
      for (const owner of missingArtifactRuntimeSignalOwners) {
        countDrillAggregateNextAction(counts, {
          owner,
          classification: "runtime-signal-coverage",
          nextAction: `include drill artifact indexes proving runtime-signal coverage owned by ${owner}`,
        })
      }
    }
    const missingArtifactOwners = checks.artifacts.missingArtifactOwners ?? []
    for (const owner of missingArtifactOwners) {
      countDrillAggregateNextAction(counts, {
        owner,
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence owned by ${owner}`,
      })
    }
    const missingArtifactClassifications = checks.artifacts.missingArtifactClassifications ?? []
    for (const classification of missingArtifactClassifications) {
      countDrillAggregateNextAction(counts, {
        owner: drillFailureOwnerForClassification(classification),
        classification,
        nextAction: `collect artifact evidence covering failure classification: ${classification}`,
      })
    }
    const missingArtifactPlannedOwners = checks.artifacts.missingArtifactPlannedOwners ?? []
    for (const owner of missingArtifactPlannedOwners) {
      countDrillAggregateNextAction(counts, {
        owner,
        classification: "matrix-coverage",
        nextAction: `include dry-run matrix artifact indexes with planned owner coverage: ${owner}`,
      })
    }
    const missingArtifactPlannedClassifications = checks.artifacts.missingArtifactPlannedClassifications ?? []
    for (const classification of missingArtifactPlannedClassifications) {
      countDrillAggregateNextAction(counts, {
        owner: drillFailureOwnerForClassification(classification),
        classification,
        nextAction: `include dry-run matrix artifact indexes with planned classification coverage: ${classification}`,
      })
    }
    const missingArtifactExitCriterionStatuses = checks.artifacts.missingArtifactExitCriterionStatuses ?? []
    if (missingArtifactExitCriterionStatuses.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence with exit criterion statuses: ${missingArtifactExitCriterionStatuses.join(", ")}`,
      })
    }
    const missingArtifactIncompleteExitCriterionStatuses = checks.artifacts.missingArtifactIncompleteExitCriterionStatuses ?? []
    if (missingArtifactIncompleteExitCriterionStatuses.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: `collect artifact evidence with incomplete exit criterion statuses: ${missingArtifactIncompleteExitCriterionStatuses.join(", ")}`,
      })
    }
    if ((checks.artifacts.staleArtifactIndexes ?? []).length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "artifact-staleness",
        nextAction: "regenerate stale drill artifact indexes, then rerun the validation gate",
        count: checks.artifacts.staleArtifactIndexes.length,
        sourceDetails: checks.artifacts.staleArtifactIndexes.map(staleArtifactIndexSourceDetail),
      })
    }
  }
  if (checks.matrices.status === "failed") {
    if (checks.matrices.error) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-artifacts",
        nextAction: "produce matrix reports under the configured matrix roots, then rerun the validation gate",
      })
    }
    for (const action of checks.matrices.aggregate?.nextActions ?? []) {
      countDrillAggregateNextAction(counts, action)
    }
    if (checks.matrices.requireComplete && (checks.matrices.aggregate?.incompleteScenarios?.length ?? 0) > 0) {
      const scenariosWithoutSpecificAction = countIncompleteScenarioPlannedActions(
        counts,
        checks.matrices.aggregate,
      )
      if (scenariosWithoutSpecificAction > 0) {
        countDrillAggregateNextAction(counts, {
          owner: "validation-harness",
          classification: "incomplete-matrix",
          nextAction: "run skipped or dry-run matrix scenarios before treating this validation set as complete",
          count: scenariosWithoutSpecificAction,
        })
      }
    }
    if (checks.matrices.requireComplete && (checks.matrices.aggregate?.incompleteExitCriteria?.length ?? 0) > 0) {
      const criteriaWithoutSpecificAction = countIncompleteExitCriterionNextActions(
        counts,
        checks.matrices.aggregate.incompleteExitCriteria,
      )
      if (criteriaWithoutSpecificAction > 0) {
        countDrillAggregateNextAction(counts, {
          owner: "validation-harness",
          classification: "incomplete-matrix",
          nextAction: "run or reconcile incomplete matrix exit criteria before treating this validation set as complete",
          count: criteriaWithoutSpecificAction,
        })
      }
    }
    const missingMatrices = checks.matrices.missingMatrices ?? []
    if (missingMatrices.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run missing drill matrices: ${missingMatrices.join(", ")}`,
      })
    }
    const missingMatrixClassifications = checks.matrices.missingMatrixClassifications ?? []
    if (missingMatrixClassifications.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports covering failure classifications: ${missingMatrixClassifications.join(", ")}`,
      })
    }
    const missingMatrixRuntimeSignals = checks.matrices.missingMatrixRuntimeSignals ?? []
    if (missingMatrixRuntimeSignals.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports covering runtime signals: ${missingMatrixRuntimeSignals.join(", ")}`,
      })
      countMissingRuntimeSignalActions(counts, missingMatrixRuntimeSignals, { target: "matrix" })
    }
    const missingDeploymentPresets = checks.matrices.missingDeploymentPresets ?? []
    if (missingDeploymentPresets.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing deployment presets: ${missingDeploymentPresets.join(", ")}`,
      })
    }
    const missingProviders = checks.matrices.missingProviders ?? []
    if (missingProviders.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing providers: ${missingProviders.join(", ")}`,
      })
    }
    const missingScenarios = checks.matrices.missingScenarios ?? []
    if (missingScenarios.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing scenarios: ${missingScenarios.join(", ")}`,
      })
    }
    if ((checks.matrices.staleMatrixReports ?? []).length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-staleness",
        nextAction: "regenerate stale matrix reports, then rerun the validation gate",
        count: checks.matrices.staleMatrixReports.length,
        sourceDetails: checks.matrices.staleMatrixReports.map(staleMatrixReportSourceDetail),
      })
    }
  }
  if (checks.failures.status === "failed") {
    if (checks.failures.error) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "failure-artifacts",
        nextAction: "fix unreadable failure artifacts or discovery configuration, then rerun the validation gate",
      })
    }
    for (const action of checks.failures.aggregate?.nextActions ?? []) {
      countDrillAggregateNextAction(counts, action)
    }
    if ((checks.failures.staleFailureManifests ?? []).length > 0) {
      const nextAction = "regenerate stale preserved failure bundles or rerun the failing drills before routing them"
      const aggregateAlreadyRoutesStaleFailures = (checks.failures.aggregate?.nextActions ?? [])
        .some((action) => action.owner === "validation-harness"
          && action.classification === "failure-artifacts"
          && action.nextAction === nextAction)
      if (!aggregateAlreadyRoutesStaleFailures) {
        countDrillAggregateNextAction(counts, {
          owner: "validation-harness",
          classification: "failure-artifacts",
          nextAction,
          count: checks.failures.staleFailureManifests.length,
          sourceDetails: checks.failures.staleFailureManifests.map(staleFailureManifestSourceDetail),
        })
      }
    }
  }
  return formatDrillAggregateNextActionCounts(counts)
}

function countMissingRuntimeSignalActions(counts, runtimeSignals, { target }) {
  for (const signal of runtimeSignals) {
    countDrillAggregateNextAction(counts, {
      owner: drillRuntimeSignalOwner(signal),
      classification: "runtime-signal-coverage",
      nextAction: drillRuntimeSignalNextAction(signal, { target }),
    })
  }
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

function staleFailureManifestSourceDetail(staleFailure) {
  return {
    source: staleFailure.drill ?? staleFailure.rootDir,
    ...(staleFailure.source ? { reportPath: staleFailure.source } : {}),
  }
}

function missingPathSourceDetail(pathValue) {
  return { source: pathValue }
}

function countIncompleteExitCriterionNextActions(counts, criteria) {
  let missingSpecificActionCount = 0
  for (const criterion of criteria) {
    if (criterion?.owner && criterion?.classification && criterion?.nextAction) {
      countDrillAggregateNextAction(counts, {
        owner: criterion.owner,
        classification: criterion.classification,
        nextAction: criterion.nextAction,
      })
    } else {
      missingSpecificActionCount += 1
    }
  }
  return missingSpecificActionCount
}

function countIncompleteScenarioPlannedActions(counts, aggregate) {
  for (const action of aggregate?.plannedNextActions ?? []) {
    countDrillAggregateNextAction(counts, {
      owner: action.owner,
      classification: action.classification,
      nextAction: action.plannedNextAction,
      count: action.count,
      ...(action.sourceDetails ? { sourceDetails: action.sourceDetails } : {}),
    })
  }
  return (aggregate?.incompleteScenarios ?? [])
    .filter((scenario) => !scenario?.plannedNextAction)
    .length
}

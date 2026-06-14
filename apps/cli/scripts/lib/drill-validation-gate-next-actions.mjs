import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
} from "./drill-aggregate-actions.mjs"

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
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "incomplete-matrix",
        nextAction: "run skipped or dry-run matrix scenarios before treating this validation set as complete",
      })
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
  }
  return formatDrillAggregateNextActionCounts(counts)
}

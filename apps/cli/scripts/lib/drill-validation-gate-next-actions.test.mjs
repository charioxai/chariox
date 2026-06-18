import assert from "node:assert/strict"
import test from "node:test"

import { validationGateNextActions } from "./drill-validation-gate-next-actions.mjs"

test("returns no next actions when every validation gate check passes or skips", () => {
  assert.deepEqual(validationGateNextActions(checks()), [])
})

test("explains missing validation gate configuration", () => {
  assert.deepEqual(validationGateNextActions(checks({
    configuration: { status: "failed", error: "no validation checks configured" },
  })), [{
    owner: "validation-harness",
    classification: "validation-gate",
    nextAction: "configure at least one platform bundle, artifact root, matrix root, or failure root before using the validation gate",
    count: 1,
  }])
})

test("explains platform bundle rebuild and missing coverage requirements", () => {
  const actions = validationGateNextActions(checks({
    platformBundle: {
      status: "failed",
      missingCoverageAreas: ["matrix-validation"],
      missingRuntimeSignals: ["slice-auth-state"],
      missingFailureClassifications: ["kernel-authority"],
    },
  }))

  assert.deepEqual(actions, [
    {
      owner: "provider-account",
      classification: "runtime-signal-coverage",
      nextAction: "add runtime-signal contract coverage for slice-auth-state owned by provider-account to the drill platform bundle",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "provide a drill platform bundle covering failure classifications: kernel-authority",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "provide a drill platform bundle covering runtime signals: slice-auth-state",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "provide a drill platform bundle covering: matrix-validation",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
      count: 1,
    },
  ])
})

test("explains artifact index failures", () => {
  assert.deepEqual(validationGateNextActions(checks({
    artifacts: { status: "failed", error: "sha256 mismatch" },
  })), [{
    owner: "validation-harness",
    classification: "artifact-index",
    nextAction: "fix missing, unreadable, or tampered artifact indexes before using collected drill evidence",
    count: 1,
  }])
})

test("explains missing executable validation suite artifacts", () => {
  assert.deepEqual(validationGateNextActions(checks({
    artifacts: {
      status: "failed",
      missingArtifactSchemas: ["arroba.drill.validation_suite_run.v1", "arroba.drill.matrix.v1"],
    },
  })), [
    {
      owner: "validation-harness",
      classification: "artifact-coverage",
      nextAction: "produce artifact evidence with schemas: arroba.drill.matrix.v1",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "artifact-coverage",
      nextAction: "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "artifact-index",
      nextAction: "fix missing, unreadable, or tampered artifact indexes before using collected drill evidence",
      count: 1,
    },
  ])
})

test("routes missing artifact diagnostic coverage to owning subsystems", () => {
  const actions = validationGateNextActions(checks({
    artifacts: {
      status: "failed",
      missingArtifactGeneratedEvidenceKinds: ["matrix-report"],
      missingArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
      missingArtifactProviderAccountAliases: ["codex=work"],
      missingArtifactRuntimeSignals: ["workspace-live-sync-state"],
      missingArtifactRuntimeSignalOwners: ["runtime-state"],
      missingArtifactOwners: ["kernel-authority"],
      missingArtifactClassifications: ["remote-extension-sync"],
      missingArtifactPlannedOwners: ["runtime-state"],
      missingArtifactPlannedClassifications: ["workspace-live-sync-conflict"],
      missingArtifactExitCriterionStatuses: ["satisfied"],
      missingArtifactIncompleteExitCriterionStatuses: ["dry-run"],
    },
  }))

  assert.equal(actions.length, 14)
  assertAction(actions, {
    owner: "provider-account",
    classification: "provider-account",
    nextAction: "include non-secret provider account alias metadata in artifact indexes: codex=work",
  })
  assertAction(actions, {
    owner: "runtime-state",
    classification: "runtime-signal-coverage",
    nextAction: "include drill artifact indexes proving workspace-live-sync-state owned by runtime-state: Workspace Live Sync mode, included paths, ignored paths, conflict/reconcile status, and peer propagation state.",
  })
  assertAction(actions, {
    owner: "runtime-state",
    classification: "matrix-coverage",
    nextAction: "include dry-run matrix artifact indexes with planned owner coverage: runtime-state",
  })
  assertAction(actions, {
    owner: "runtime-state",
    classification: "workspace-live-sync-conflict",
    nextAction: "include dry-run matrix artifact indexes with planned classification coverage: workspace-live-sync-conflict",
  })
  assertAction(actions, {
    owner: "kernel-authority",
    classification: "remote-extension-sync",
    nextAction: "collect artifact evidence covering failure classification: remote-extension-sync",
  })
  assertAction(actions, {
    owner: "kernel-authority",
    classification: "artifact-coverage",
    nextAction: "collect artifact evidence owned by kernel-authority",
  })
  assertAction(actions, {
    owner: "validation-harness",
    classification: "artifact-coverage",
    nextAction: "collect artifact evidence with incomplete exit criterion statuses: dry-run",
  })
  assertAction(actions, {
    owner: "validation-harness",
    classification: "generated-evidence",
    nextAction: "provide generated drill evidence with kinds: matrix-report",
  })
})

test("explains matrix errors, incomplete scenarios, missing coverage, and aggregate actions", () => {
  const actions = validationGateNextActions(checks({
    matrices: {
      status: "failed",
      error: "no matrix reports found",
      requireComplete: true,
      missingMatrices: ["workspace-live-sync-matrix"],
      missingMatrixClassifications: ["workspace-live-sync-conflict"],
      missingMatrixRuntimeSignals: ["lease-health", "workspace-live-sync-state"],
      missingDeploymentPresets: ["hosted-cloud"],
      missingProviders: ["claude"],
      missingScenarios: ["tracked"],
      aggregate: {
        incompleteScenarios: [
          {
            id: "tracked",
            status: "dry-run",
            plannedClassification: "workspace-live-sync-conflict",
            plannedOwner: "runtime-state",
            plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
          },
          { id: "hetzner", status: "skipped" },
        ],
        incompleteExitCriteria: [
          {
            id: "auth:exit-01",
            status: "failed",
            owner: "provider-account",
            classification: "provider-auth",
            nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
          },
          { id: "tracked:exit-01", status: "dry-run" },
        ],
        nextActions: [{
          owner: "provider-account",
          classification: "provider-auth",
          nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
          count: 2,
        }],
        plannedNextActions: [{
          owner: "runtime-state",
          classification: "workspace-live-sync-conflict",
          plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
          count: 1,
          sourceDetails: [{
            source: "workspace/tracked",
            matrix: "workspace",
            scenarioId: "tracked",
            reportPath: "/tmp/workspace-matrix.json",
          }],
        }],
      },
    },
  }))

  assert.deepEqual(actions, [
    {
      owner: "provider-account",
      classification: "provider-auth",
      nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
      count: 3,
    },
    {
      owner: "kernel-authority",
      classification: "runtime-signal-coverage",
      nextAction: "run matrix scenarios that prove lease-health owned by kernel-authority: Remote leased-agent session, worker, reconnect, and provider-run binding health.",
      count: 1,
    },
    {
      owner: "runtime-state",
      classification: "runtime-signal-coverage",
      nextAction: "run matrix scenarios that prove workspace-live-sync-state owned by runtime-state: Workspace Live Sync mode, included paths, ignored paths, conflict/reconcile status, and peer propagation state.",
      count: 1,
    },
    {
      owner: "runtime-state",
      classification: "workspace-live-sync-conflict",
      nextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
      count: 1,
      sourceDetails: [{
        source: "workspace/tracked",
        matrix: "workspace",
        scenarioId: "tracked",
        reportPath: "/tmp/workspace-matrix.json",
      }],
    },
    {
      owner: "validation-harness",
      classification: "incomplete-matrix",
      nextAction: "run or reconcile incomplete matrix exit criteria before treating this validation set as complete",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "incomplete-matrix",
      nextAction: "run skipped or dry-run matrix scenarios before treating this validation set as complete",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-artifacts",
      nextAction: "produce matrix reports under the configured matrix roots, then rerun the validation gate",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports covering failure classifications: workspace-live-sync-conflict",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports covering runtime signals: lease-health, workspace-live-sync-state",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing deployment presets: hosted-cloud",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing providers: claude",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing scenarios: tracked",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run missing drill matrices: workspace-live-sync-matrix",
      count: 1,
    },
  ])
})

test("explains failure artifact errors and forwards preserved failure actions", () => {
  const actions = validationGateNextActions(checks({
    failures: {
      status: "failed",
      error: "unsupported schema",
      aggregate: {
        nextActions: [{
          owner: "provider-account",
          classification: "provider-auth",
          nextAction: "refresh provider login for the profile used by this drill, then rerun the drill",
          count: 1,
        }],
      },
    },
  }))

  assert.deepEqual(actions, [
    {
      owner: "provider-account",
      classification: "provider-auth",
      nextAction: "refresh provider login for the profile used by this drill, then rerun the drill",
      count: 1,
    },
    {
      owner: "validation-harness",
      classification: "failure-artifacts",
      nextAction: "fix unreadable failure artifacts or discovery configuration, then rerun the validation gate",
      count: 1,
    },
  ])
})

function checks(overrides = {}) {
  return {
    configuration: { status: "passed" },
    platformBundle: { status: "skipped" },
    artifacts: { status: "skipped" },
    matrices: { status: "skipped" },
    failures: { status: "skipped" },
    ...overrides,
  }
}

function assertAction(actions, expected) {
  assert(actions.some((action) => (
    action.owner === expected.owner
    && action.classification === expected.classification
    && action.nextAction === expected.nextAction
    && action.count === (expected.count ?? 1)
  )), `missing action ${JSON.stringify(expected)}`)
}

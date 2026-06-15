import assert from "node:assert/strict"
import test from "node:test"

import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./drill-validation-gate-args.mjs"

test("parses validation gate requirement arguments", () => {
  const options = validationGateRequirementOptionDefaults()
  let index = parseValidationGateRequirementArg(["--preset", "workspace-live-sync"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-platform-coverage-area=runtime-fixtures"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-artifact-coverage-area", "distributed-observability"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-schema", "arroba.drill.validation_suite_run.v1"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-kind=validation-suite-run"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-artifact-generated-evidence-kind", "validation-suite-run"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-generated-matrix-artifact-index", "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-generated-matrix-limitation", "dry-run-classification-coverage"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-evidence-repo", "oss"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-provider-account-alias", "codex=work"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-runtime-signal", "session-authority"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-runtime-signal-owner=kernel-authority"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-artifact-owner", "validation-platform"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-artifact-classification=validation-gate"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-artifact-exit-criterion-status=satisfied"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-artifact-incomplete-exit-criterion-status", "dry-run"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-provider", "codex"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-runtime-signal", "lease-health"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-matrix-runtime-signal=workspace-live-sync-state"], 0, options)
  assert.equal(index, 0)
  index = parseValidationGateRequirementArg(["--require-generated-evidence-kind", "matrix-report"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-generated-matrix-artifact-index", "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-generated-matrix-limitation", "dry-run-classification-coverage"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--require-generated-validation-suite-failure-root", "/tmp/generated-suite/failed-run"], 0, options)
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--unknown"], 0, options)
  assert.equal(index, null)

  assert.deepEqual(options, {
    presets: ["workspace-live-sync"],
    requiredPlatformCoverageAreas: ["runtime-fixtures"],
    requiredArtifactCoverageAreas: ["distributed-observability"],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    requiredArtifactKinds: ["validation-suite-run"],
    requiredArtifactGeneratedEvidenceKinds: ["validation-suite-run"],
    requiredArtifactGeneratedMatrixArtifactIndexes: ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
    requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    requiredArtifactEvidenceRepos: ["oss"],
    requiredArtifactProviderAccountAliases: ["codex=work"],
    requiredArtifactRuntimeSignals: ["session-authority"],
    requiredArtifactRuntimeSignalOwners: ["kernel-authority"],
    requiredArtifactOwners: ["validation-platform"],
    requiredArtifactClassifications: ["validation-gate"],
    requiredArtifactExitCriterionStatuses: ["satisfied"],
    requiredArtifactIncompleteExitCriterionStatuses: ["dry-run"],
    requiredRuntimeSignals: ["lease-health"],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
    requiredDeploymentPresets: [],
    requiredProviders: ["codex"],
    requiredScenarios: [],
    requiredGeneratedEvidenceKinds: ["matrix-report"],
    requiredGeneratedMatrixArtifactIndexes: ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
    requiredGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    requiredGeneratedValidationSuiteFailureRoots: ["/tmp/generated-suite/failed-run"],
  })
})

test("parses aggregate preset requirements with custom key", () => {
  const options = validationGateRequirementOptionDefaults({ presetKey: "requiredPresets" })
  const index = parseValidationGateRequirementArg(
    ["--require-preset=remote-home-extension"],
    0,
    options,
    {
      presetFlag: "--require-preset",
      presetKey: "requiredPresets",
    },
  )

  assert.equal(index, 0)
  assert.deepEqual(options.requiredPresets, ["remote-home-extension"])
  assert.deepEqual(options.requiredProviders, [])
})

test("parses requirement arguments without accepting a preset flag", () => {
  const options = validationGateRequirementOptionDefaults()
  let index = parseValidationGateRequirementArg(
    ["--require-runtime-signal", "session-authority"],
    0,
    options,
    { presetFlag: null },
  )
  assert.equal(index, 1)
  index = parseValidationGateRequirementArg(["--preset", "workspace-live-sync"], 0, options, { presetFlag: null })
  assert.equal(index, null)
  assert.deepEqual(options.presets, [])
  assert.deepEqual(options.requiredRuntimeSignals, ["session-authority"])
})

test("rejects missing validation gate requirement values", () => {
  const options = validationGateRequirementOptionDefaults()
  assert.throws(
    () => parseValidationGateRequirementArg(["--require-provider"], 0, options),
    /--require-provider requires a value/,
  )
  assert.throws(
    () => parseValidationGateRequirementArg(["--preset", "--json"], 0, options),
    /--preset requires a value/,
  )
})

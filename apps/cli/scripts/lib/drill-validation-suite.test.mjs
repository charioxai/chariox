import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_COVERAGE_AREAS,
  SHARED_DRILL_TEST_PATHS,
  discoverDrillValidationSuiteTestPaths,
  drillValidationSuiteArtifactMetadata,
  drillValidationSuiteArgs,
  drillValidationSuiteCommand,
  drillValidationSuiteManifest,
  findMissingDrillValidationSuitePaths,
  findUnlistedDrillValidationSuitePaths,
  normalizeValidationSuitePresetContracts,
  validateValidationSuiteCoverage,
  validationSuiteCoverage,
} from "./drill-validation-suite.mjs"
import {
  DRILL_RUNTIME_SIGNAL_IDS,
  drillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"

test("shared drill validation suite lists stable test paths", () => {
  assert(SHARED_DRILL_TEST_PATHS.includes("apps/cli/scripts/lib/drill-matrix-runner.test.mjs"))
  assert(SHARED_DRILL_TEST_PATHS.includes("apps/cli/scripts/drill-matrix-report-summary.test.mjs"))
  assert.deepEqual([...SHARED_DRILL_TEST_PATHS].sort(), [...SHARED_DRILL_TEST_PATHS])
  assert.equal(new Set(SHARED_DRILL_TEST_PATHS).size, SHARED_DRILL_TEST_PATHS.length)
})

test("shared drill validation suite covers every test path exactly once", () => {
  validateValidationSuiteCoverage()
  const covered = validationSuiteCoverage().flatMap((area) => area.testPaths)
  assert.deepEqual([...covered].sort(), [...SHARED_DRILL_TEST_PATHS])
  assert.equal(new Set(covered).size, SHARED_DRILL_TEST_PATHS.length)
  assert.deepEqual(
    DRILL_VALIDATION_COVERAGE_AREAS.map((area) => area.id),
    ["distributed-observability", "artifact-contracts", "failure-diagnostics", "matrix-validation", "runtime-fixtures", "suite-contract"],
  )
})

test("shared drill validation suite includes every CLI script test", async () => {
  const discovered = await discoverDrillValidationSuiteTestPaths()
  assert.deepEqual(discovered, [...SHARED_DRILL_TEST_PATHS])
})

test("rejects invalid shared drill validation coverage", () => {
  assert.throws(() => validateValidationSuiteCoverage({
    coverageAreas: [{
      id: "bad",
      description: "bad coverage",
      testPaths: ["apps/cli/scripts/lib/missing.test.mjs"],
    }],
    testPaths: ["apps/cli/scripts/lib/drill-validation-suite.test.mjs"],
  }), /references non-suite test/)
  assert.throws(() => validateValidationSuiteCoverage({
    coverageAreas: [{
      id: "bad",
      description: "bad coverage",
      testPaths: [],
    }],
    testPaths: ["apps/cli/scripts/lib/drill-validation-suite.test.mjs"],
  }), /invalid testPaths/)
  assert.throws(() => validateValidationSuiteCoverage({
    coverageAreas: [{
      id: "bad",
      description: "bad coverage",
      testPaths: ["apps/cli/scripts/lib/drill-validation-suite.test.mjs"],
    }],
    testPaths: [
      "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
      "apps/cli/scripts/lib/drill-time.test.mjs",
    ],
  }), /missing coverage areas/)
})

test("formats shared drill validation suite command", () => {
  assert.deepEqual(drillValidationSuiteArgs({ testPaths: ["one.test.mjs", "two words.test.mjs"] }), [
    "--test",
    "one.test.mjs",
    "two words.test.mjs",
  ])
  assert.equal(
    drillValidationSuiteCommand({ nodeCommand: "node", testPaths: ["one.test.mjs", "two words.test.mjs"] }),
    'node --test one.test.mjs "two words.test.mjs"',
  )
})

test("builds shared drill validation suite manifest", () => {
  assert.deepEqual(drillValidationSuiteManifest({
    nodeCommand: "node",
    testPaths: ["one.test.mjs", "two words.test.mjs"],
    coverageAreas: [{
      id: "sample",
      description: "sample coverage",
      testPaths: ["one.test.mjs", "two words.test.mjs"],
    }],
    validationPresets: [{
      name: "sample-preset",
      description: "sample preset",
      requiredMatrices: ["sample-matrix"],
    }],
  }), {
    schema: "arroba.drill.validation_suite.v1",
    testCount: 2,
    command: 'node --test one.test.mjs "two words.test.mjs"',
    coverage: [{
      id: "sample",
      description: "sample coverage",
      testCount: 2,
      testPaths: ["one.test.mjs", "two words.test.mjs"],
    }],
    runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    validationPresets: [{
      name: "sample-preset",
      description: "sample preset",
      requiredPlatformCoverageAreas: [],
      requiredArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      requiredArtifactKinds: [],
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactGeneratedMatrixLimitations: [],
      requiredArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      requiredArtifactClassifications: [],
      requiredRuntimeSignals: [],
      requiredFailureClassifications: [],
      requiredMatrices: ["sample-matrix"],
      requiredMatrixClassifications: [],
      requiredMatrixRuntimeSignals: [],
      requiredDeploymentPresets: [],
      requiredProviders: [],
      requiredScenarios: [],
    }],
    testPaths: ["one.test.mjs", "two words.test.mjs"],
  })
})

test("builds validation suite artifact metadata from manifest and run report", () => {
  const manifest = drillValidationSuiteManifest()
  assert.deepEqual(manifest.runtimeSignalsManifest, drillRuntimeSignalsManifest())

  assert.deepEqual(drillValidationSuiteArtifactMetadata(manifest), {
    drill: "validation-suite",
    tests: SHARED_DRILL_TEST_PATHS.length,
    owners: "validation-platform",
    classifications: "validation-suite",
    artifactKinds: "validation-suite",
    evidenceRepos: "oss",
    coverageAreas: "artifact-contracts,distributed-observability,failure-diagnostics,matrix-validation,runtime-fixtures,suite-contract",
    validationPresets: "distributed-runtime,native-provider-tui,remote-agent-runtime,remote-home-extension,slice-runtime,workspace-live-sync",
    runtimeSignals: DRILL_RUNTIME_SIGNAL_IDS.join(","),
    runtimeSignalOwners: "kernel-authority,provider-account,provider-runtime,runtime-network,runtime-state,ui-client,worker-kernel",
  })
  assert.deepEqual(drillValidationSuiteArtifactMetadata({
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    manifest,
  }), {
    drill: "validation-suite",
    status: "passed",
    tests: SHARED_DRILL_TEST_PATHS.length,
    owners: "validation-platform",
    classifications: "validation-suite",
    artifactKinds: "validation-suite-run",
    evidenceRepos: "oss",
    coverageAreas: "artifact-contracts,distributed-observability,failure-diagnostics,matrix-validation,runtime-fixtures,suite-contract",
    validationPresets: "distributed-runtime,native-provider-tui,remote-agent-runtime,remote-home-extension,slice-runtime,workspace-live-sync",
    runtimeSignals: DRILL_RUNTIME_SIGNAL_IDS.join(","),
    runtimeSignalOwners: "kernel-authority,provider-account,provider-runtime,runtime-network,runtime-state,ui-client,worker-kernel",
  })
  assert.throws(() => drillValidationSuiteArtifactMetadata({
    ...manifest,
    runtimeSignalsManifest: {
      ...drillRuntimeSignalsManifest(),
      signals: drillRuntimeSignalsManifest().signals.filter((signal) => signal.id !== "lease-health"),
    },
  }), /validation suite runtimeSignalsManifest does not match required runtime signals/)
  assert.throws(() => drillValidationSuiteArtifactMetadata(null), /requires a manifest or run report/)
})

test("normalizes validation suite preset contracts", () => {
  assert.deepEqual(normalizeValidationSuitePresetContracts([{
    name: "workspace-live-sync",
    description: "Workspace Live Sync",
    requiredPlatformCoverageAreas: ["runtime-fixtures", "matrix-validation", "matrix-validation"],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    requiredArtifactKinds: ["validation-suite-run", "validation-suite-run"],
    requiredArtifactGeneratedEvidenceKinds: ["matrix-report", "matrix-report"],
    requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage", "dry-run-classification-coverage"],
    requiredArtifactEvidenceRepos: ["oss", "cloud", "oss"],
    requiredArtifactRuntimeSignals: ["workspace-live-sync-state", "session-authority", "workspace-live-sync-state"],
    requiredArtifactRuntimeSignalOwners: ["runtime-state"],
    requiredArtifactOwners: ["validation-platform"],
    requiredArtifactClassifications: ["validation-suite"],
    requiredMatrices: ["workspace-live-sync-matrix"],
    requiredFailureClassifications: ["workspace-live-sync-conflict", "kernel-authority"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    requiredDeploymentPresets: ["local", "hetzner", "local"],
    requiredProviders: ["opencode", "codex", "codex"],
  }]), [{
    name: "workspace-live-sync",
    description: "Workspace Live Sync",
    requiredPlatformCoverageAreas: ["matrix-validation", "runtime-fixtures"],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    requiredArtifactKinds: ["validation-suite-run"],
    requiredArtifactGeneratedEvidenceKinds: ["matrix-report"],
    requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    requiredArtifactEvidenceRepos: ["cloud", "oss"],
    requiredArtifactRuntimeSignals: ["session-authority", "workspace-live-sync-state"],
    requiredArtifactRuntimeSignalOwners: ["runtime-state"],
    requiredArtifactOwners: ["validation-platform"],
    requiredArtifactClassifications: ["validation-suite"],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: ["kernel-authority", "workspace-live-sync-conflict"],
    requiredMatrices: ["workspace-live-sync-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: ["hetzner", "local"],
    requiredProviders: ["codex", "opencode"],
    requiredScenarios: [],
  }])
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad",
    description: "bad",
    requiredMatrices: "matrix",
  }]), /requiredMatrices must be an array/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-runtime-signal",
    description: "bad runtime signal",
    requiredRuntimeSignals: ["workspace-live-synch-state"],
  }]), /requiredRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-artifact-runtime-signal",
    description: "bad artifact runtime signal",
    requiredArtifactRuntimeSignals: ["workspace-live-synch-state"],
  }]), /requiredArtifactRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-matrix-runtime-signal",
    description: "bad matrix runtime signal",
    requiredMatrixRuntimeSignals: ["workspace-live-synch-state"],
  }]), /requiredMatrixRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-runtime-signal-owner",
    description: "bad runtime signal owner",
    requiredArtifactRuntimeSignalOwners: ["runtime-stat"],
  }]), /requiredArtifactRuntimeSignalOwners\[0\] has unknown runtime signal owner "runtime-stat"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-artifact-kind",
    description: "bad artifact kind",
    requiredArtifactKinds: ["validation-sutie"],
  }]), /requiredArtifactKinds\[0\] has unknown artifact kind "validation-sutie"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-generated-evidence-kind",
    description: "bad generated evidence kind",
    requiredArtifactGeneratedEvidenceKinds: ["matrix-reprot"],
  }]), /requiredArtifactGeneratedEvidenceKinds\[0\] has unknown generated evidence kind "matrix-reprot"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-generated-matrix-limitation",
    description: "bad generated matrix limitation",
    requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-covergae"],
  }]), /requiredArtifactGeneratedMatrixLimitations\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-artifact-evidence-repo",
    description: "bad artifact evidence repo",
    requiredArtifactEvidenceRepos: ["clodu"],
  }]), /requiredArtifactEvidenceRepos\[0\] has unknown artifact evidence repo "clodu"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-failure-classification",
    description: "bad failure classification",
    requiredFailureClassifications: ["kernel-autohority"],
  }]), /requiredFailureClassifications\[0\] has unknown failure classification "kernel-autohority"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-matrix-classification",
    description: "bad matrix classification",
    requiredMatrixClassifications: ["kernel-autohority"],
  }]), /requiredMatrixClassifications\[0\] has unknown failure classification "kernel-autohority"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-deployment-preset",
    description: "bad deployment preset",
    requiredDeploymentPresets: ["self-hotsed-relay"],
  }]), /requiredDeploymentPresets\[0\] has unknown deployment preset "self-hotsed-relay"/)
  assert.throws(() => normalizeValidationSuitePresetContracts([{
    name: "bad-provider",
    description: "bad provider",
    requiredProviders: ["cdoex"],
  }]), /requiredProviders\[0\] has unknown provider "cdoex"/)
})

test("finds missing shared drill validation suite paths", async () => {
  assert.deepEqual(await findMissingDrillValidationSuitePaths({
    testPaths: ["apps/cli/scripts/lib/drill-validation-suite.test.mjs"],
  }), [])
  assert.deepEqual(await findMissingDrillValidationSuitePaths({
    testPaths: ["apps/cli/scripts/lib/missing-suite-test.mjs"],
  }), ["apps/cli/scripts/lib/missing-suite-test.mjs"])
})

test("finds unlisted shared drill validation suite paths", async () => {
  assert.deepEqual(await findUnlistedDrillValidationSuitePaths({
    testPaths: SHARED_DRILL_TEST_PATHS.filter((testPath) => testPath !== "apps/cli/scripts/lib/drill-validation-suite.test.mjs"),
  }), ["apps/cli/scripts/lib/drill-validation-suite.test.mjs"])
})

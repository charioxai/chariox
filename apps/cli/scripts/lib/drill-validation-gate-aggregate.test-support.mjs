import assert from "node:assert/strict"
import test from "node:test"

import {
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"


export function generatedEvidenceFixture() {
  return {
    validationSuites: {
      enabled: true,
      artifactIndexes: [
        "/tmp/suites/cloud/chariox-drill-artifacts.json",
        "/tmp/suites/oss/chariox-drill-artifacts.json",
      ],
      failureRoots: [
        "/tmp/suites/cloud/failed-run",
        "/tmp/suites/oss/failed-run",
      ],
      commands: [
        {
          artifactIndexPath: "/tmp/suites/oss/chariox-drill-artifacts.json",
          args: ["--run-json", "--preserve-failure-root", "/tmp/suites/oss/failed-run"],
          cwd: "/repo/chariox",
          failureRoot: "/tmp/suites/oss/failed-run",
          nodeArgs: ["/repo/chariox/apps/cli/scripts/drill-validation-suite.mjs", "--run-json", "--output", "/tmp/suites/oss/drill-validation-suite-run.json", "--output-artifact-index", "/tmp/suites/oss/chariox-drill-artifacts.json", "--preserve-failure-root", "/tmp/suites/oss/failed-run"],
          reportPath: "/tmp/suites/oss/drill-validation-suite-run.json",
          scriptPath: "/repo/chariox/apps/cli/scripts/drill-validation-suite.mjs",
        },
        {
          artifactIndexPath: "/tmp/suites/cloud/chariox-drill-artifacts.json",
          args: ["--run-json", "--preserve-failure-root", "/tmp/suites/cloud/failed-run"],
          cwd: "/repo/chariox-cloud",
          failureRoot: "/tmp/suites/cloud/failed-run",
          nodeArgs: ["/repo/chariox-cloud/scripts/cloud-validation-suite.mjs", "--run-json", "--output", "/tmp/suites/cloud/cloud-validation-suite-run.json", "--output-artifact-index", "/tmp/suites/cloud/chariox-drill-artifacts.json", "--preserve-failure-root", "/tmp/suites/cloud/failed-run"],
          reportPath: "/tmp/suites/cloud/cloud-validation-suite-run.json",
          scriptPath: "/repo/chariox-cloud/scripts/cloud-validation-suite.mjs",
        },
      ],
      outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
    },
    matrixReports: {
      enabled: true,
      artifactIndexes: ["/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json"],
      roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
      dryRun: false,
      continueOnFailure: true,
      limitations: [{
        kind: "dry-run-classification-coverage",
        owner: "validation-harness",
        nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before release",
      }],
      commands: [{
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        args: ["--include-hetzner"],
        artifactIndexFlag: "--artifact-index",
        cwd: "/repo/chariox",
        matrix: "native-provider-tui-matrix",
        nodeArgs: ["/repo/chariox/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs", "--include-hetzner", "--report", "/tmp/matrices/oss/native-provider-tui-matrix.json", "--artifact-index", "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json"],
        repo: "oss",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/chariox/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
    },
  }
}

export function reportFixture(overrides = {}) {
  return {
    schema: "chariox.drill.validation_gate.v1",
    status: "passed",
    presets: ["workspace-live-sync"],
    checks: {
      configuration: { status: "passed" },
      platformBundle: {
        status: "passed",
        requiredCoverageAreas: ["runtime-fixtures"],
        missingCoverageAreas: [],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: [],
      },
      artifacts: {
        status: "passed",
        requiredArtifactSchemas: ["chariox.drill.validation_suite_run.v1"],
        missingArtifactSchemas: [],
        requiredArtifactKinds: ["validation-suite-run"],
        missingArtifactKinds: [],
        requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
        missingArtifactGeneratedMatrixLimitations: [],
        requiredArtifactEvidenceRepos: ["oss"],
        missingArtifactEvidenceRepos: [],
        requiredArtifactProviderAccountAliases: ["codex=work"],
        missingArtifactProviderAccountAliases: [],
        requiredArtifactValidationPresets: ["distributed-runtime"],
        missingArtifactValidationPresets: [],
        requiredArtifactRuntimeAuthorityInvariants: ["home-session-authority"],
        missingArtifactRuntimeAuthorityInvariants: [],
        aggregate: {
          schemas: {
            "chariox.drill.validation_suite_run.v1": 1,
          },
          runtimeSignals: {
            "session-authority": 2,
            "workspace-live-sync-state": 1,
          },
          runtimeSignalOwners: {
            "kernel-authority": 1,
            "runtime-state": 1,
          },
          runtimeAuthorityInvariants: {
            "home-session-authority": 1,
          },
          requiredRuntimeAuthorityInvariants: {
            "home-session-authority": 1,
          },
          missingRuntimeAuthorityInvariants: {},
          owners: {
            "validation-platform": 1,
          },
          classifications: {
            "cloud-validation-suite": 1,
          },
          exitCriterionStatuses: {
            "dry-run": 1,
          },
          incompleteExitCriterionStatuses: {
            "dry-run": 1,
          },
          artifactKinds: {
            "validation-suite-run": 1,
          },
          generatedMatrixLimitations: {
            "dry-run-classification-coverage": 1,
          },
          generatedValidationSuiteArtifactIndexes: {
            "/tmp/generated-suite/chariox-drill-artifacts.json": 1,
          },
          generatedValidationSuiteFailureRoots: {
            "/tmp/generated-suite/failed-run": 1,
          },
          evidenceRepos: {
            oss: 1,
          },
          providerAccountAliases: {
            "codex=work": 1,
          },
          validationPresets: {
            "distributed-runtime": 1,
          },
          artifactCoverageInputSources: {
            "artifact metadata inputs": 1,
          },
        },
      },
      matrices: {
        status: "passed",
        requiredMatrices: ["workspace-live-sync-matrix"],
        missingMatrices: [],
        requiredMatrixClassifications: ["workspace-live-sync-conflict"],
        missingMatrixClassifications: [],
        requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
        missingMatrixRuntimeSignals: [],
        aggregate: {
          owners: {
            "runtime-state": 1,
          },
          classifications: {
            "workspace-live-sync-conflict": 1,
          },
          runtimeSignals: {
            "workspace-live-sync-state": 1,
          },
          runtimeSignalScenarios: {
            "workspace-live-sync-state": [{
              matrix: "workspace-live-sync-matrix",
              source: "/tmp/workspace-live-sync-matrix.json",
              id: "managed",
              status: "passed",
            }],
          },
        },
        requiredDeploymentPresets: ["local"],
        missingDeploymentPresets: [],
        requiredProviders: ["codex"],
        missingProviders: [],
        requiredScenarios: ["managed"],
        missingScenarios: [],
      },
      failures: { status: "skipped" },
    },
    nextActions: [],
    ...overrides,
  }
}

export {
  assert,
  test,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
}

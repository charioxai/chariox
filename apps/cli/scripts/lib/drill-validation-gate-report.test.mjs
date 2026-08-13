import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"

test("accepts a minimal passed validation gate report", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report()))
})

test("rejects reports with unsupported schema or mismatched top-level status", () => {
  assert.throws(
    () => validateDrillValidationGateReport({ ...report(), schema: "wrong" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport({
      ...report({ checks: { artifacts: { status: "failed", roots: [], inputs: [], indexPaths: [], error: "missing" } } }),
      status: "passed",
    }),
    /status does not match check statuses/,
  )
})

test("rejects unknown validation gate preset labels in reports", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({ presets: ["workspace-live-synch"] })),
    /presets\[0\] has unknown validation gate preset "workspace-live-synch"/,
  )
})

test("rejects invalid configuration and next-action records", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: { configuration: { status: "skipped" } },
    })),
    /checks\.configuration cannot be skipped/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      nextActions: [{ owner: "validation-harness", classification: "validation-gate", nextAction: "" }],
    })),
    /nextActions\[0\] is missing nextAction/,
  )
})

test("validates optional generated evidence provenance", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report({
    generatedEvidence: generatedEvidence(),
  })))
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          enabled: false,
          artifactIndexes: ["/tmp/artifacts.json"],
          failureRoots: [],
          commands: [],
          outputRoots: [],
        },
      },
    })),
    /generatedEvidence\.validationSuites disabled evidence has paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          enabled: true,
          artifactIndexes: [],
          failureRoots: [],
          commands: [],
          outputRoots: [],
        },
      },
    })),
    /generatedEvidence\.validationSuites enabled evidence is missing paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            args: [],
            artifactIndexFlag: "--artifact-index",
            artifactIndexPath: "/tmp/matrix-artifacts.json",
            cwd: "/repo/chariox",
            reportPath: "",
            scriptPath: "/repo/chariox/matrix.mjs",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\] has invalid reportPath/,
  )
  {
    const { artifactIndexFlag: _artifactIndexFlag, ...commandWithoutFlag } = generatedEvidence().matrixReports.commands[0]
    assert.throws(
      () => validateDrillValidationGateReport(report({
        generatedEvidence: {
          ...generatedEvidence(),
          matrixReports: {
            ...generatedEvidence().matrixReports,
            commands: [commandWithoutFlag],
          },
        },
      })),
      /generatedEvidence\.matrixReports\.commands\[0\] has invalid artifactIndexFlag/,
    )
  }
  {
    const { nodeArgs: _nodeArgs, ...commandWithoutNodeArgs } = generatedEvidence().validationSuites.commands[0]
    assert.throws(
      () => validateDrillValidationGateReport(report({
        generatedEvidence: {
          ...generatedEvidence(),
          validationSuites: {
            ...generatedEvidence().validationSuites,
            commands: [commandWithoutNodeArgs],
          },
        },
      })),
      /generatedEvidence\.validationSuites\.commands\[0\]\.nodeArgs is not an array/,
    )
  }
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            ...generatedEvidence().matrixReports.commands[0],
            matrix: "",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\] has invalid matrix/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            ...generatedEvidence().matrixReports.commands[0],
            matrix: "workspace-live-synch-matrix",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\]\.matrix has unknown generated matrix name "workspace-live-synch-matrix"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            ...generatedEvidence().matrixReports.commands[0],
            repo: "osz",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\]\.repo has unknown evidence repo "osz"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            ...generatedEvidence().matrixReports.commands[0],
            matrix: "cloud-slice-runtime-matrix",
            repo: "oss",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\]\.repo does not match generated matrix "cloud-slice-runtime-matrix"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          enabled: true,
          artifactIndexes: [],
          roots: [],
          commands: [],
          dryRun: false,
          continueOnFailure: false,
        },
      },
    })),
    /generatedEvidence\.matrixReports enabled evidence is missing paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          enabled: false,
          artifactIndexes: [],
          roots: ["/tmp/matrices"],
          commands: [],
          dryRun: false,
          continueOnFailure: false,
        },
      },
    })),
    /generatedEvidence\.matrixReports disabled evidence has generated data/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          dryRun: true,
          limitations: [],
        },
      },
    })),
    /generatedEvidence\.matrixReports dry-run evidence is missing limitations/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          limitations: [{ kind: "dry-run-classification-coverage", owner: "", nextAction: "rerun live matrix reports" }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.limitations\[0\] has invalid owner/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          limitations: [{ kind: "dry-run-classification-covergae", owner: "validation-harness", nextAction: "rerun live matrix reports" }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.limitations\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          ...generatedEvidence().validationSuites,
          failureRoots: ["/tmp/Bearer abcdefghijklmnop"],
        },
      },
    })),
    /generatedEvidence\.validationSuites\.failureRoots\[0\] includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          ...generatedEvidence().validationSuites,
          commands: [{
            ...generatedEvidence().validationSuites.commands[0],
            args: ["--preserve-failure-root", "/tmp/Bearer abcdefghijklmnop"],
          }],
        },
      },
    })),
    /generatedEvidence\.validationSuites\.commands\[0\]\.args\[1\] includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          artifactIndexes: ["/tmp/Bearer abcdefghijklmnop.json"],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.artifactIndexes\[0\] includes secret-looking generated evidence path/,
  )
})

test("validates platform bundle summary evidence", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report({
    checks: {
      platformBundle: {
        status: "passed",
        dir: "/tmp/platform",
        requiredCoverageAreas: [],
        missingCoverageAreas: [],
        requiredFailureClassifications: [],
        missingFailureClassifications: [],
        artifacts: [{
          path: "validation-suite.json",
          schema: "chariox.drill.validation_suite.v1",
          sha256: "a".repeat(64),
          sizeBytes: 10,
        }],
        validationSuite: {
          testCount: 2,
          coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
        },
        failureTaxonomy: {
          drill: ["kernel-authority"],
          scenario: ["kernel-authority"],
        },
      },
    },
  })))
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        platformBundle: {
          status: "passed",
          dir: "/tmp/platform",
          requiredCoverageAreas: [],
          missingCoverageAreas: [],
          requiredFailureClassifications: [],
          missingFailureClassifications: [],
          artifacts: [{
            path: "validation-suite.json",
            schema: "chariox.drill.validation_suite.v1",
            sha256: "bad",
            sizeBytes: 10,
          }],
          validationSuite: {
            testCount: 2,
            coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
          },
        },
      },
    })),
    /artifacts\[0\] has invalid sha256/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        platformBundle: {
          status: "passed",
          dir: "/tmp/platform",
          requiredCoverageAreas: [],
          missingCoverageAreas: [],
          requiredFailureClassifications: [],
          missingFailureClassifications: [],
          artifacts: [],
          validationSuite: {
            testCount: 3,
            coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
          },
        },
      },
    })),
    /coverageAreas do not match testCount/,
  )
})

test("validates aggregate schemas for artifact, matrix, and failure checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.artifacts\.aggregate has unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.matrices\.aggregate has unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        failures: {
          status: "passed",
          roots: [],
          inputs: [],
          manifestPaths: [],
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.failures\.aggregate has unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          aggregate: { schema: "chariox.drill.artifact_index.aggregate.v1" },
        },
      },
    })),
    /checks\.artifacts\.aggregate has invalid totals/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          aggregate: {
            ...matrixAggregate(),
            runtimeSignals: { "session-authority": 2 },
          },
        },
      },
    })),
    /checks\.matrices\.aggregate runtimeSignals do not match runtimeSignalScenarios/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        failures: {
          status: "failed",
          roots: [],
          inputs: [],
          manifestPaths: [],
          aggregate: {
            ...failureAggregate(),
            runtimeSignals: { "lease-health": 2 },
            runtimeSignalOwners: { "kernel-authority": 2 },
          },
        },
      },
    })),
    /checks\.failures\.aggregate runtimeSignals do not match failures/,
  )
})

test("rejects unknown artifact evidence repo labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactGeneratedEvidenceRepos: ["cluod"],
          missingArtifactGeneratedEvidenceRepos: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactGeneratedEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactGeneratedEvidenceRepos: [],
          missingArtifactGeneratedEvidenceRepos: ["cluod"],
        },
      },
    })),
    /checks\.artifacts\.missingArtifactGeneratedEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactEvidenceRepos: ["cluod"],
          missingArtifactEvidenceRepos: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactEvidenceRepos: [],
          missingArtifactEvidenceRepos: ["cluod"],
        },
      },
    })),
    /checks\.artifacts\.missingArtifactEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
})

test("rejects unknown artifact provider account alias labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactProviderAccountAliases: ["cdoex=work"],
          missingArtifactProviderAccountAliases: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactProviderAccountAliases\[0\] has unknown provider account alias provider "cdoex"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactProviderAccountAliases: [],
          missingArtifactProviderAccountAliases: ["codex=sk-secretsecretsecretsecret"],
        },
      },
    })),
    /provider account alias must be a non-secret label/,
  )
})

test("rejects unknown generated evidence kind labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactGeneratedEvidenceKinds: ["matrix-reprot"],
          missingArtifactGeneratedEvidenceKinds: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactGeneratedEvidenceKinds\[0\] has unknown generated evidence kind "matrix-reprot"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-covergae"],
          missingArtifactGeneratedMatrixLimitations: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactGeneratedMatrixLimitations\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        kinds: ["matrix-reprot"],
      },
    })),
    /generatedEvidence\.kinds\[0\] has unknown generated evidence kind "matrix-reprot"/,
  )
})

test("rejects unknown artifact kind labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactKinds: ["validation-sutie"],
          missingArtifactKinds: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactKinds\[0\] has unknown artifact kind "validation-sutie"/,
  )
})

test("rejects unknown provider labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          requiredProviders: ["cdoex"],
        },
      },
    })),
    /checks\.matrices\.requiredProviders\[0\] has unknown provider "cdoex"/,
  )
})

test("rejects unknown deployment preset labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          requiredDeploymentPresets: ["same-host-remtoe"],
        },
      },
    })),
    /checks\.matrices\.requiredDeploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/,
  )
})

test("rejects unknown runtime signal and classification labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        platformBundle: {
          status: "passed",
          dir: "/tmp/platform",
          requiredCoverageAreas: [],
          missingCoverageAreas: [],
          requiredRuntimeSignals: ["workspace-live-synch-state"],
          missingRuntimeSignals: [],
          requiredFailureClassifications: [],
          missingFailureClassifications: [],
          artifacts: [],
          validationSuite: { testCount: 0, coverageAreas: [] },
        },
      },
    })),
    /checks\.platformBundle\.requiredRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactRuntimeSignalOwners: ["kernel-authorit"],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactRuntimeSignalOwners\[0\] has unknown runtime signal owner "kernel-authorit"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          requiredMatrixClassifications: ["workspace-live-synch-conflict"],
        },
      },
    })),
    /checks\.matrices\.requiredMatrixClassifications\[0\] has unknown failure classification "workspace-live-synch-conflict"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          requiredMatrixRuntimeSignals: ["workspace-live-synch-state"],
        },
      },
    })),
    /checks\.matrices\.requiredMatrixRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.doesNotThrow(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: matrixCheck({
          requiredMatrixRuntimeSignals: ["session-authority"],
          requiredMatrixRuntimeSignalScenarios: {
            "session-authority": [{
              matrix: "remote-agent-runtime-matrix",
              source: "/tmp/matrix.json",
              id: "single-user-remote-agent",
              status: "passed",
            }],
          },
        }),
      },
    })),
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: matrixCheck({
          requiredMatrixRuntimeSignals: ["session-authority"],
          requiredMatrixRuntimeSignalScenarios: {},
        }),
      },
    })),
    /checks\.matrices\.requiredMatrixRuntimeSignalScenarios is missing required signal "session-authority"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: matrixCheck({
          requiredMatrixRuntimeSignalScenarios: {
            "workspace-live-synch-state": [],
          },
        }),
      },
    })),
    /checks\.matrices\.requiredMatrixRuntimeSignalScenarios has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: matrixCheck({
          requiredMatrixRuntimeSignals: ["session-authority"],
          requiredMatrixRuntimeSignalScenarios: {
            "session-authority": [{
              matrix: "remote-agent-runtime-matrix",
              source: "/tmp/Bearer abcdefghijklmnop/matrix.json",
              id: "single-user-remote-agent",
              status: "passed",
            }],
          },
        }),
      },
    })),
    /checks\.matrices\.requiredMatrixRuntimeSignalScenarios\.session-authority\[0\] includes secret-looking source/,
  )
})

function report(overrides = {}) {
  const checks = {
    configuration: { status: "passed" },
    platformBundle: {
      status: "skipped",
      dir: null,
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    },
    artifacts: { status: "skipped", roots: [], inputs: [], indexPaths: [] },
    matrices: matrixCheck(),
    failures: { status: "skipped", roots: [], inputs: [], manifestPaths: [] },
    ...(overrides.checks ?? {}),
  }
  return {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
    presets: [],
    checks,
    nextActions: [],
    ...overrides,
    checks,
  }
}

function generatedEvidence() {
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
      commands: [{
        args: ["--include-hetzner"],
        artifactIndexFlag: "--artifact-index",
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        cwd: "/repo/chariox",
        matrix: "native-provider-tui-matrix",
        nodeArgs: ["/repo/chariox/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs", "--include-hetzner", "--report", "/tmp/matrices/oss/native-provider-tui-matrix.json", "--artifact-index", "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json"],
        repo: "oss",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/chariox/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
      dryRun: false,
      continueOnFailure: true,
      limitations: [],
    },
  }
}

function matrixCheck(overrides = {}) {
  return {
    status: "skipped",
    roots: [],
    inputs: [],
    reportPaths: [],
    requireComplete: false,
    requiredMatrices: [],
    missingMatrices: [],
    requiredMatrixClassifications: [],
    missingMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    missingMatrixRuntimeSignals: [],
    requiredMatrixRuntimeSignalScenarios: {},
    requiredDeploymentPresets: [],
    missingDeploymentPresets: [],
    requiredProviders: [],
    missingProviders: [],
    requiredScenarios: [],
    missingScenarios: [],
    ...overrides,
  }
}

function matrixAggregate() {
  return {
    schema: "chariox.drill.matrix.aggregate.v1",
    status: "passed",
    totals: { reports: 1, scenarios: 1, passed: 1, failed: 0, skipped: 0, dryRun: 0, durationMs: 10 },
    failedScenarios: [],
    skippedScenarios: [],
    incompleteScenarios: [],
    owners: {},
    classifications: {},
    matrixNames: { "test-matrix": 1 },
    deploymentPresets: {},
    providers: {},
    scenarioIds: { local: 1 },
    runtimeSignals: { "session-authority": 1 },
    runtimeSignalScenarios: {
      "session-authority": [{
        matrix: "test-matrix",
        source: "/tmp/matrix.json",
        id: "local",
        status: "passed",
      }],
    },
    nextActions: [],
    reports: [{
      matrix: "test-matrix",
      source: "/tmp/matrix.json",
      status: "passed",
      classifications: {},
      deploymentPresets: [],
      providers: [],
      scenarioIds: ["local"],
      runtimeSignals: { "session-authority": 1 },
      runtimeSignalScenarios: {
        "session-authority": [{
          id: "local",
          status: "passed",
        }],
      },
      scenarioCount: 1,
      counts: { passed: 1, failed: 0, skipped: 0, dryRun: 0 },
      durationMs: 10,
    }],
  }
}

function failureAggregate() {
  return {
    schema: "chariox.drill.failure.aggregate.v1",
    total: 1,
    owners: { "runtime-network": 1 },
    classifications: { "relay-runtime": 1 },
    runtimeSignals: { "lease-health": 1 },
    runtimeSignalOwners: { "kernel-authority": 1 },
    nextActions: [{
      owner: "runtime-network",
      classification: "relay-runtime",
      nextAction: "inspect relay and kernel logs in the preserved artifact root, then rerun the drill",
      count: 1,
    }],
    failures: [{
      drill: "relay-drill",
      rootDir: "/tmp/failure",
      owner: "runtime-network",
      classification: "relay-runtime",
      runtimeSignals: ["lease-health"],
      nextAction: "inspect relay and kernel logs in the preserved artifact root, then rerun the drill",
    }],
  }
}

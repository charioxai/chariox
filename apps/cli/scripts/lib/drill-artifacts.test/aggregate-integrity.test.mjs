import {
  assert,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
  os,
  path,
  test,
  DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
  DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS,
  DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
  DRILL_ARTIFACT_INDEX_SCHEMA,
  diagnosticMetadataForDrillArtifactIndexAggregate,
  emptyDrillArtifactDiagnosticDimensions,
  findDrillArtifactIndexPaths,
  focusedRuntimeGateReportArtifact,
  formatDrillArtifactIndexAggregateSummary,
  matrixReportArtifact,
  prepareDrillArtifacts,
  readDrillArtifactIndex,
  summarizeDrillArtifactIndexes,
  validateDrillArtifactDiagnosticDimensions,
  validateDrillArtifactIndex,
  validateDrillArtifactIndexAggregate,
  validationGateReportArtifact,
  validationSuiteManifestArtifact,
  validationSuiteRunArtifact,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
  finalizeDrillArtifacts,
} from '../drill-artifacts.test-support.mjs'

test("rejects inconsistent drill artifact index aggregates", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "gate.json"), "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/gate.json"],
    })
    const aggregate = summarizeDrillArtifactIndexes([index])

    assert.doesNotThrow(() => validateDrillArtifactIndexAggregate({
      ...aggregate,
      nextActions: [{
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: "include drill artifact indexes with required coverage",
        count: 1,
      }],
    }))
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        nextActions: "invalid",
      }),
      /nextActions is invalid/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        nextActions: [{
          owner: "validation-harness",
          classification: "artifact-coverage",
          nextAction: "retry with Bearer abcdefghijklmnopqrstuvwxyz",
          count: 1,
        }],
      }),
      /nextActions\[0\] includes secret-looking nextAction/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        totals: {
          ...aggregate.totals,
          artifacts: aggregate.totals.artifacts + 1,
        },
      }),
      /totals do not match indexes/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        schemas: { "arroba.drill.matrix.v1": 1 },
      }),
      /schemas do not match indexes/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        generatedEvidenceKinds: { "matrix-report": 1 },
      }),
      /generatedEvidenceKinds do not match indexes/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        generatedMatrixArtifactIndexes: { "/tmp/Bearer abcdefghijklmnop.json": 1 },
      }),
      /generatedMatrixArtifactIndexes\.\/tmp\/Bearer abcdefghijklmnop\.json includes secret-looking generated evidence path/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        providerAccountAliases: { "cdoex=work": 1 },
      }),
      /providerAccountAliases has unknown provider account alias provider "cdoex"/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        runtimeSignals: { "workspace-live-synch-state": 1 },
      }),
      /runtimeSignals has unknown runtime signal "workspace-live-synch-state"/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        indexes: [{
          ...aggregate.indexes[0],
          runtimeSignals: { "workspace-live-synch-state": 1 },
          runtimeSignalOwners: { "runtime-state": 1 },
        }],
      }),
      /indexes\[0\]\.runtimeSignals has unknown runtime signal "workspace-live-synch-state"/,
    )
    assert.throws(
      () => validateDrillArtifactIndexAggregate({
        ...aggregate,
        runtimeSignals: { "workspace-live-sync-state": 1 },
        runtimeSignalOwners: { "kernel-authority": 1 },
        indexes: [{
          ...aggregate.indexes[0],
          runtimeSignals: { "workspace-live-sync-state": 1 },
          runtimeSignalOwners: { "kernel-authority": 1 },
        }],
      }),
      /indexes\[0\]\.runtimeSignalOwners must match runtimeSignals/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects invalid drill artifact diagnostic dimensions", () => {
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions({
      runtimeSignals: {},
      runtimeSignalOwners: {},
      coverageAreas: {},
      validationPresets: {},
      owners: {},
      classifications: {},
      exitCriterionStatuses: {},
      incompleteExitCriterionStatuses: {},
      artifactKinds: {},
    }),
    /missing requiredRuntimeSignals/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      evidenceRepos: { oss: -1 },
    })),
    /evidenceRepos has invalid count/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      runtimeSignalOwners: { "kernel-authority": 1 },
    })),
    /drill artifact diagnostics\.runtimeSignalOwners must match runtimeSignals/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      runtimeSignals: { "workspace-live-synch-state": 1 },
    })),
    /runtimeSignals has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredRuntimeSignals: { "workspace-live-synch-state": 1 },
    })),
    /requiredRuntimeSignals has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredRuntimeSignalOwners: { "kernel-authority": 1 },
    })),
    /drill artifact diagnostics\.requiredRuntimeSignalOwners must match requiredRuntimeSignals/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      missingRuntimeSignals: { "workspace-live-synch-state": 1 },
    })),
    /missingRuntimeSignals has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      missingRuntimeSignals: { "provider-run-lifecycle": 1 },
      missingRuntimeSignalOwners: { "kernel-authority": 1 },
    })),
    /drill artifact diagnostics\.missingRuntimeSignalOwners must match missingRuntimeSignals/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      exitCriterionStatuses: { satisifed: 1 },
    })),
    /exitCriterionStatuses has unknown exit criterion status "satisifed"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      evidenceRepos: { cluod: 1 },
    })),
    /evidenceRepos has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedEvidenceRepos: { cluod: 1 },
    })),
    /generatedEvidenceRepos has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedEvidenceKinds: { "matrix-reprot": 1 },
    })),
    /generatedEvidenceKinds has unknown generated evidence kind "matrix-reprot"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      validationPresets: { "distributed-runtmie": 1 },
    })),
    /validationPresets has unknown validation preset "distributed-runtmie"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredFailureClassifications: { "provider-aut": 1 },
    })),
    /requiredFailureClassifications has unknown failure classification "provider-aut"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedValidationSuiteFailureRoots: { "/tmp/Bearer abcdefghijklmnop": 1 },
    })),
    /generatedValidationSuiteFailureRoots\.\/tmp\/Bearer abcdefghijklmnop includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredGeneratedMatrixArtifactIndexes: { "/tmp/Bearer abcdefghijklmnop.json": 1 },
    })),
    /requiredGeneratedMatrixArtifactIndexes\.\/tmp\/Bearer abcdefghijklmnop\.json includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedMatrixLimitations: { "dry-run-classification-covergae": 1 },
    })),
    /generatedMatrixLimitations has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredGeneratedMatrixLimitations: { "dry-run-classification-covergae": 1 },
    })),
    /requiredGeneratedMatrixLimitations has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      missingGeneratedMatrixLimitations: { "dry-run-classification-covergae": 1 },
    })),
    /missingGeneratedMatrixLimitations has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredGeneratedMatrixNames: { "Bearer abcdefghijklmnop": 1 },
    })),
    /requiredGeneratedMatrixNames\.Bearer abcdefghijklmnop includes secret-looking generated matrix name/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      missingGeneratedMatrixNames: { "Bearer abcdefghijklmnop": 1 },
    })),
    /missingGeneratedMatrixNames\.Bearer abcdefghijklmnop includes secret-looking generated matrix name/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedMatrixNames: { "workspace-live-synch-matrix": 1 },
    })),
    /generatedMatrixNames has unknown generated matrix name "workspace-live-synch-matrix"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      generatedMatrixNames: { "cloud-slice-runtime-matrix": 1 },
      generatedMatrixRepos: { oss: 1 },
    })),
    /generated matrix "cloud-slice-runtime-matrix" without generated matrix repo "cloud"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      requiredGeneratedMatrixRepos: { osz: 1 },
    })),
    /requiredGeneratedMatrixRepos has unknown evidence repo "osz"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      missingGeneratedMatrixRepos: { osz: 1 },
    })),
    /missingGeneratedMatrixRepos has unknown evidence repo "osz"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      artifactKinds: { "validation-sutie": 1 },
    })),
    /artifactKinds has unknown artifact kind "validation-sutie"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      providerAccountAliases: { "cdoex=work": 1 },
    })),
    /providerAccountAliases has unknown provider account alias provider "cdoex"/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions(emptyDrillArtifactDiagnosticDimensions({
      providerAccountAliases: { "codex=sk-secretsecretsecretsecret": 1 },
    })),
    /provider account alias must be a non-secret label/,
  )
})

test("rejects unsafe drill artifact index paths", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: "",
        artifacts: ["reports/gate.json"],
      }),
      /rootDir is required/,
    )
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["../outside.json"],
      }),
      /escapes root/,
    )

    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: "relative-root",
        createdAt: new Date().toISOString(),
        metadata: {},
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /invalid rootDir/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: {},
        artifacts: [{
          path: "../outside.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /unsafe path/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { evidenceRepos: "cluod" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.evidenceRepos has unknown evidence repo "cluod"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedEvidenceRepos: "cluod" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedEvidenceRepos has unknown evidence repo "cluod"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedValidationSuiteFailureRoots: "/tmp/Bearer abcdefghijklmnop" },
        artifacts: [{
          path: "reports/gate.json",
          schema: "arroba.drill.validation_gate.v1",
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedValidationSuiteFailureRoots\[0\] includes secret-looking generated evidence path/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedEvidenceKinds: "matrix-reprot" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedEvidenceKinds has unknown generated evidence kind "matrix-reprot"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedMatrixLimitations: "dry-run-classification-covergae" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedMatrixLimitations has unknown generated matrix limitation "dry-run-classification-covergae"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedMatrixRepos: "cluod" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedMatrixRepos has unknown evidence repo "cluod"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { requiredGeneratedMatrixRepos: "osz" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.requiredGeneratedMatrixRepos has unknown evidence repo "osz"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { missingGeneratedMatrixRepos: "osz" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.missingGeneratedMatrixRepos has unknown evidence repo "osz"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedMatrixNames: "Bearer abcdefghijklmnop" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedMatrixNames\[0\] includes secret-looking generated matrix name/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { requiredGeneratedMatrixNames: "Bearer abcdefghijklmnop" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.requiredGeneratedMatrixNames\[0\] includes secret-looking generated matrix name/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { missingGeneratedMatrixNames: "Bearer abcdefghijklmnop" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.missingGeneratedMatrixNames\[0\] includes secret-looking generated matrix name/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { generatedMatrixNames: "workspace-live-synch-matrix" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedMatrixNames has unknown generated matrix name "workspace-live-synch-matrix"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: {
          generatedMatrixNames: "cloud-slice-runtime-matrix",
          generatedMatrixRepos: "oss",
        },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.generatedMatrixNames has generated matrix "cloud-slice-runtime-matrix" without generated matrix repo "cloud"/,
    )
    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: { artifactKinds: "validation-sutie" },
        artifacts: [{
          path: "reports/gate.json",
          schema: null,
          sha256: "0".repeat(64),
          sizeBytes: 0,
        }],
      }),
      /metadata\.artifactKinds has unknown artifact kind "validation-sutie"/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects duplicate drill artifact index records", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "gate.json"), "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/gate.json", "reports/gate.json"],
      }),
      /duplicate artifact reports\/gate\.json/,
    )

    assert.throws(
      () => validateDrillArtifactIndex({
        schema: DRILL_ARTIFACT_INDEX_SCHEMA,
        rootDir: root,
        createdAt: new Date().toISOString(),
        metadata: {},
        artifacts: [
          {
            path: "reports/gate.json",
            schema: null,
            sha256: "0".repeat(64),
            sizeBytes: 0,
          },
          {
            path: "reports/gate.json",
            schema: null,
            sha256: "0".repeat(64),
            sizeBytes: 0,
          },
        ],
      }),
      /duplicate artifact reports\/gate\.json/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("verifies drill artifact index integrity", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    const reportPath = path.join(root, "reports", "gate.json")
    await writeFile(reportPath, `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
      status: "passed",
    })}\n`, "utf8")
    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(reportPath, `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
      status: "failed",
    })}\n`, "utf8")

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /sha256 mismatch/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

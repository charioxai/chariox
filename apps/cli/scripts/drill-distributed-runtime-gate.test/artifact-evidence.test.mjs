import {
  assert,
  DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO,
  DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS,
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  drillRuntimeAuthorityManifest,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  execFile,
  generatedMatrixNamesForEvidenceRepo,
  matrixReport,
  mkdtemp,
  os,
  path,
  readFile,
  rm,
  scenario,
  scriptPath,
  summaryScriptPath,
  test,
  verifyDrillArtifactIndex,
  writeCloudFailureTaxonomyRegistry,
  writeCloudGeneratedMatrixRegistry,
  writeCloudRuntimeAuthorityRegistry,
  writeCloudRuntimeSignalsRegistry,
  writeDistributedRuntimeMatrices,
  writeDrillArtifactIndex,
  writeFailureManifest,
  writeFakeDistributedRuntimeMatrixScripts,
  writeFakeMatrixScript,
  writeFakeValidationSuiteScript,
  writeValidationSuiteArtifact,
  writeValidationSuiteManifestArtifact,
} from '../drill-distributed-runtime-gate.test-support.mjs'

test("distributed runtime gate help lists artifact evidence requirements", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /--require-artifact-generated-matrix-name NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-generated-evidence-repo REPO\[,REPO\]/)
  assert.match(stdout, /--require-artifact-generated-matrix-repo REPO\[,REPO\]/)
  assert.match(stdout, /--require-artifact-generated-validation-suite-artifact-index PATH\[,PATH\]/)
  assert.match(stdout, /--require-artifact-validation-preset NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-planned-owner OWNER\[,OWNER\]/)
  assert.match(stdout, /--require-artifact-planned-classification KIND\[,KIND\]/)
  assert.match(stdout, /--require-runtime-authority-registry-parity/)
})

test("distributed runtime gate accepts a pnpm argument separator", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--", "--help"])

  assert.match(stdout, /Runs the release-level distributed-runtime validation gate/)
  assert.match(stdout, /--require-generated-matrix-registry-parity/)
})

test("distributed runtime gate passes with complete OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
      providerAccountAliases: "codex=work",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      providerAccountAliases: "opencode=zen",
      plannedOwners: "validation-platform",
      plannedClassifications: "workspace-live-sync-conflict",
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-complete",
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
      "--require-artifact-provider-account-alias",
      "codex=work",
      "--require-artifact-provider-account-alias",
      "opencode=zen",
      "--require-artifact-validation-preset",
      "cloud-distributed-runtime",
      "--require-artifact-planned-owner",
      "validation-platform",
      "--require-artifact-planned-classification",
      "workspace-live-sync-conflict",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, report)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.generatedEvidence, {
      matrixReports: {
        artifactIndexes: [],
        commands: [],
        continueOnFailure: false,
        dryRun: false,
        enabled: false,
        limitations: [],
        roots: [],
      },
      validationSuites: {
        artifactIndexes: [],
        commands: [],
        enabled: false,
        failureRoots: [],
        outputRoots: [],
      },
    })
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.equal(report.checks.artifacts.aggregate.coverageAreas["distributed-observability"], 2)
    assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactKinds, ["validation-suite-run"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(report.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work", "opencode=zen"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, {
      "codex=work": 1,
      "opencode=zen": 1,
    })
    assert.deepEqual(report.checks.artifacts.requiredArtifactValidationPresets, ["cloud-distributed-runtime", "distributed-runtime", "distributed-state-health", "runtime-authority"])
    assert.deepEqual(report.checks.artifacts.missingArtifactValidationPresets, [])
    assert.deepEqual(report.checks.artifacts.aggregate.validationPresets, {
      "cloud-distributed-runtime": 1,
      "distributed-runtime": 1,
      "distributed-state-health": 1,
      "runtime-authority": 1,
    })
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedOwners, ["validation-platform"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedOwners, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedClassifications, ["workspace-live-sync-conflict"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedClassifications, [])
    assert.deepEqual(report.presets, ["distributed-runtime"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.ok(report.checks.platformBundle.requiredRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.ok(report.checks.matrices.requiredMatrixRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.aggregate.runtimeSignalScenarios["slice-auth-state"].map((entry) => entry.id), ["provider-auth"])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.deepEqual(
      report.checks.matrices.aggregate.reports.find((entry) => entry.matrix === "cloud-slice-runtime-matrix").providers,
      ["claude", "codex", "opencode"],
    )
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 4)
    assert.equal(artifactIndex.metadata.drill, "distributed-runtime-gate")
    assert.equal(artifactIndex.metadata.preset, "distributed-runtime")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work,opencode=zen")
    assert.equal(artifactIndex.metadata.matrixEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.artifactEvidenceRepos, "cloud,oss")
    const indexedCoverageAreas = artifactIndex.metadata.coverageAreas.split(",")
    assert.equal(indexedCoverageAreas.includes("distributed-observability"), true)
    assert.equal(indexedCoverageAreas.includes("suite-contract"), true)
    assert.equal(indexedCoverageAreas.includes("matrix-validation"), true)
    const indexedRuntimeSignals = artifactIndex.metadata.runtimeSignals.split(",")
    assert.equal(indexedRuntimeSignals.includes("home-extension-manifest-sync"), true)
    assert.equal(indexedRuntimeSignals.includes("provider-run-lifecycle"), true)
    assert.equal(indexedRuntimeSignals.includes("runtime-projection-health"), true)
    assert.equal(indexedRuntimeSignals.includes("runtime-transition-audit"), true)
    assert.equal(indexedRuntimeSignals.includes("slice-auth-state"), true)
    assert.equal(indexedRuntimeSignals.includes("workspace-live-sync-state"), true)
    const indexedRuntimeSignalOwners = artifactIndex.metadata.runtimeSignalOwners.split(",")
    assert.equal(indexedRuntimeSignalOwners.includes("kernel-authority"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-account"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-runtime"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("runtime-state"), true)
    const indexedClassifications = artifactIndex.metadata.classifications.split(",")
    assert.equal(indexedClassifications.includes("kernel-authority"), true)
    assert.equal(indexedClassifications.includes("remote-extension-sync"), true)
    assert.equal(indexedClassifications.includes("workspace-live-sync-conflict"), true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires default artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        return true
      },
    )

    const discovered = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(discovered.status, "passed")
    assert.equal(discovered.checks.artifacts.status, "passed")
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactSchemas, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.deepEqual(discovered.checks.artifacts.aggregate.evidenceRepos, { cloud: 1, oss: 1 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate accepts explicit artifact evidence inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const explicitArtifactRoot = path.join(rootDir, "validation-artifacts", "cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    const ossArtifactIndex = await writeValidationSuiteArtifact(path.join(rootDir, "validation-artifacts", "oss"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(explicitArtifactRoot)

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      ossArtifactIndex,
      "--artifact-root",
      explicitArtifactRoot,
      "--json",
    ])).stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.roots, [explicitArtifactRoot])
    assert.deepEqual(report.checks.artifacts.inputs, [ossArtifactIndex])
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can run validation suites as artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--json",
    ])).stdout)

    const expectedArtifactIndexes = [
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
    ]
    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, expectedArtifactIndexes)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
    assert.deepEqual(report.generatedEvidence.validationSuites, {
      artifactIndexes: expectedArtifactIndexes.sort(),
      failureRoots: [
        path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
        path.join(validationSuiteOutputRoot, "oss", "failed-run"),
      ].sort(),
      commands: [
        {
          artifactIndexPath: path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
          args: [
            "--run-json",
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          ],
          cwd: ossRoot,
          failureRoot: path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          nodeArgs: [
            path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
            "--run-json",
            "--output",
            path.join(validationSuiteOutputRoot, "oss", "drill-validation-suite-run.json"),
            "--output-artifact-index",
            path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          ],
          reportPath: path.join(validationSuiteOutputRoot, "oss", "drill-validation-suite-run.json"),
          scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
        },
        {
          artifactIndexPath: path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
          args: [
            "--run-json",
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          ],
          cwd: cloudRoot,
          failureRoot: path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          nodeArgs: [
            path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
            "--run-json",
            "--output",
            path.join(validationSuiteOutputRoot, "cloud", "cloud-validation-suite-run.json"),
            "--output-artifact-index",
            path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          ],
          reportPath: path.join(validationSuiteOutputRoot, "cloud", "cloud-validation-suite-run.json"),
          scriptPath: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
        },
      ],
      enabled: true,
      outputRoots: [
        path.join(validationSuiteOutputRoot, "cloud"),
        path.join(validationSuiteOutputRoot, "oss"),
      ].sort(),
    })
    assert.equal(report.generatedEvidence.matrixReports.enabled, false)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can run matrix reports as evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const matrixOutputRoot = path.join(rootDir, "generated-matrices")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    const outputPath = path.join(rootDir, "distributed-runtime-gate.json")
    const artifactIndexPath = path.join(rootDir, "distributed-runtime-gate-artifacts.json")
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })
    await writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--run-matrix-reports",
      "--matrix-output-root",
      matrixOutputRoot,
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--require-artifact-provider-account-alias",
      "codex=work_codex",
      "--require-artifact-provider-account-alias",
      "opencode=zen",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      "--json",
    ])).stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, report)
    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work_codex", "opencode=zen"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, {
      "claude=work_claude": 6,
      "codex=work_codex": 7,
      "opencode=zen": 7,
    })
    assert.equal(report.checks.matrices.status, "passed")
    assert.deepEqual(report.checks.matrices.roots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort())
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["browser-terminal-resilience-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["runtime-resilience-chaos-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["workspace-live-sync-matrix"], 1)
    assert.equal(report.generatedEvidence.matrixReports.enabled, true)
    assert.deepEqual(report.generatedEvidence.matrixReports.limitations, [])
    assert.deepEqual(report.generatedEvidence.matrixReports.roots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort())
    assert.deepEqual(report.generatedEvidence.matrixReports.artifactIndexes, [
      path.join(matrixOutputRoot, "cloud", "browser-terminal-resilience-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-agent-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-home-extension-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "runtime-resilience-chaos-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json"),
    ])
    assert.equal(report.generatedEvidence.matrixReports.commands.length, 8)
    assert.equal(report.generatedEvidence.matrixReports.commands[0].reportPath, path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix.json"))
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[0].nodeArgs, [
      path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
      "--report",
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix.json"),
      "--artifact-index",
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[7].nodeArgs.slice(-4), [
      "--report",
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix.json"),
      "--output-artifact-index",
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[0].args, [
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[1].args, [
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
      "--include-hosted-cloud",
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[2].args, [
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-slices",
      "--include-hetzner",
      "--include-hosted-cloud",
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[3].args, ["--include-hetzner"])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[5].args, [
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-remote",
      "--include-hetzner",
      "--include-opencode",
    ])
    assert.deepEqual(report.generatedEvidence.validationSuites.artifactIndexes, [
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
    ].sort())
    assert.equal(artifactIndex.metadata.generatedValidationSuiteArtifactIndexes, [
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
    ].sort().join(","))
    assert.equal(artifactIndex.metadata.generatedMatrixArtifactIndexes, [
      path.join(matrixOutputRoot, "cloud", "browser-terminal-resilience-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-agent-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-home-extension-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "runtime-resilience-chaos-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json"),
    ].join(","))
    assert.equal(artifactIndex.metadata.generatedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.generatedMatrixNames, [
      "browser-terminal-resilience-matrix",
      "cloud-slice-runtime-matrix",
      "native-provider-tui-matrix",
      "remote-agent-runtime-matrix",
      "remote-home-extension-matrix",
      "runtime-resilience-chaos-matrix",
      "slice-runtime-matrix",
      "workspace-live-sync-matrix",
    ].join(","))
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.generatedEvidenceRepos, "cloud,external,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "claude=work_claude,codex=work_codex,opencode=zen")
    assert.equal(artifactIndex.metadata.generatedMatrixRoots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort().join(","))
    assert.equal(artifactIndex.metadata.generatedValidationSuiteRoots, [
      path.join(validationSuiteOutputRoot, "cloud"),
      path.join(validationSuiteOutputRoot, "oss"),
    ].sort().join(","))
    assert.equal(artifactIndex.metadata.generatedValidationSuiteFailureRoots, [
      path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
      path.join(validationSuiteOutputRoot, "oss", "failed-run"),
    ].sort().join(","))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate labels dry-run generated matrix limitations", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const matrixOutputRoot = path.join(rootDir, "generated-matrices")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    const outputPath = path.join(rootDir, "distributed-runtime-gate.json")
    const artifactIndexPath = path.join(rootDir, "distributed-runtime-gate-artifacts.json")
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })
    await writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--run-matrix-reports",
      "--matrix-dry-run",
      "--matrix-output-root",
      matrixOutputRoot,
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      "--json",
    ])).stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    const summary = JSON.parse((await execFile(process.execPath, [
      summaryScriptPath,
      "--gate-report",
      outputPath,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--require-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
    ])).stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.generatedEvidence.matrixReports.dryRun, true)
    assert.deepEqual(report.generatedEvidence.matrixReports.limitations, [{
      kind: "dry-run-classification-coverage",
      owner: "validation-harness",
      nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before treating required matrix classifications as release evidence",
    }])
    assert.equal(artifactIndex.metadata.generatedMatrixLimitations, "dry-run-classification-coverage")
    assert.match(artifactIndex.metadata.generatedMatrixNames, /cloud-slice-runtime-matrix/)
    assert.match(artifactIndex.metadata.generatedMatrixNames, /runtime-resilience-chaos-matrix/)
    assert.match(artifactIndex.metadata.generatedMatrixNames, /workspace-live-sync-matrix/)
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "cloud,oss")
    assert.match(artifactIndex.metadata.generatedMatrixArtifactIndexes, /browser-terminal-resilience-matrix-artifacts\.json/)
    assert.match(artifactIndex.metadata.generatedMatrixArtifactIndexes, /cloud-slice-runtime-matrix-artifacts\.json/)
    assert.match(artifactIndex.metadata.generatedMatrixArtifactIndexes, /workspace-live-sync-matrix-artifacts\.json/)
    assert.equal(summary.status, "passed")
    assert.equal(summary.coverage.generatedMatrixArtifactIndexes[path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json")], 1)
    assert.equal(summary.coverage.generatedMatrixArtifactIndexes[path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json")], 1)
    assert.deepEqual(summary.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(summary.missingArtifactGeneratedMatrixLimitations, [])
    assert.deepEqual(summary.requiredGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(summary.missingGeneratedMatrixLimitations, [])
    assert(report.generatedEvidence.matrixReports.commands.every((command) => command.args.includes("--dry-run")))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

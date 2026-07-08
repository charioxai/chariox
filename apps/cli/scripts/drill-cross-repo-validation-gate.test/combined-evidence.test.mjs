import {
  assert,
  execFile,
  mkdtemp,
  os,
  path,
  readFile,
  rm,
  scriptPath,
  test,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillPlatformBundle,
  writeFile,
  scenario,
  writeCloudFailureTaxonomyRegistry,
  writeCloudGeneratedMatrixRegistry,
  writeCloudRuntimeAuthorityRegistry,
  writeCloudRuntimeSignalsRegistry,
  writeFailureManifest,
  writeMatrixReport,
  writeValidationSuiteArtifact,
} from '../drill-cross-repo-validation-gate.test-support.mjs'

test("cross repo validation gate help lists artifact identity requirements", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /--require-artifact-generated-matrix-name NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-generated-matrix-repo REPO\[,REPO\]/)
  assert.match(stdout, /--require-artifact-generated-validation-suite-artifact-index PATH\[,PATH\]/)
  assert.match(stdout, /--require-artifact-validation-preset NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-planned-owner OWNER\[,OWNER\]/)
  assert.match(stdout, /--require-artifact-planned-classification KIND\[,KIND\]/)
  assert.match(stdout, /--require-runtime-authority-registry-parity/)
})

test("cross repo validation gate combines OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(ossRoot, ".artifacts", "drill-matrices", "slice-runtime.json"), {
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
        scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
        scenario("session-start", "kernel-authority", ["session-authority"]),
        scenario("agent-reuse", "worker-execution", ["agent-lifecycle"]),
        scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      ],
    })
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: {
        plannedClassifications: "workspace-live-sync-conflict",
        plannedOwners: "validation-platform",
        providerAccountAliases: "codex=work",
        validationPresets: "cloud-distributed-runtime",
      },
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--platform-bundle",
      bundleDir,
      "--preset",
      "slice-runtime",
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
      "--require-artifact-provider-account-alias",
      "codex=work",
      "--require-artifact-validation-preset",
      "cloud-distributed-runtime",
      "--require-artifact-planned-owner",
      "validation-platform",
      "--require-artifact-planned-classification",
      "workspace-live-sync-conflict",
      "--require-complete",
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
    assert.equal(report.checks.artifacts.status, "passed")
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, { "codex=work": 1 })
    assert.deepEqual(report.checks.artifacts.requiredArtifactValidationPresets, ["cloud-distributed-runtime"])
    assert.deepEqual(report.checks.artifacts.missingArtifactValidationPresets, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedOwners, ["validation-platform"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedOwners, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedClassifications, ["workspace-live-sync-conflict"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedClassifications, [])
    assert.deepEqual(report.checks.artifacts.aggregate.indexes.map((index) => path.relative(cloudRoot, index.rootDir)), [
      path.join(".artifacts", "validation-suite"),
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["cloud-slice-runtime-matrix", "slice-runtime-matrix"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingMatrixClassifications, [])
    assert.deepEqual(report.checks.platformBundle.requiredRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "runtime-projection-health",
      "runtime-transition-audit",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
    ])
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.requiredMatrixRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "runtime-projection-health",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
    ])
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.aggregate.runtimeSignalScenarios["slice-auth-state"].map((entry) => entry.id), ["provider-auth"])
    assert.equal(report.checks.matrices.aggregate.matrixNames["slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.deepEqual(
      report.checks.matrices.aggregate.reports.find((entry) => entry.matrix === "cloud-slice-runtime-matrix").providers,
      ["claude", "codex", "opencode"],
    )
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["local"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["self-hosted-relay"], 1)
    assert.equal(artifactIndex.metadata.drill, "cross-repo-validation-gate")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work")
    assert.equal(artifactIndex.metadata.matrixEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.artifactEvidenceRepos, "cloud")
    assertMetadataIncludes(artifactIndex.metadata.runtimeSignals, [
      "agent-lifecycle",
      "home-extension-manifest-sync",
      "provider-run-lifecycle",
      "session-authority",
      "slice-auth-state",
      "workspace-live-sync-state",
    ])
    assertMetadataIncludes(artifactIndex.metadata.runtimeSignalOwners, [
      "kernel-authority",
      "provider-account",
      "provider-runtime",
      "runtime-state",
      "ui-client",
      "worker-kernel",
    ])
    assertMetadataIncludes(artifactIndex.metadata.classifications, [
      "docker-runtime",
      "kernel-authority",
      "remote-extension-sync",
      "slice-auth",
      "slice-runtime",
      "ui-client-projection",
      "workspace-live-sync-conflict",
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

function assertMetadataIncludes(value, expected) {
  const actual = new Set(String(value ?? "").split(",").filter(Boolean))
  for (const entry of expected) {
    assert.equal(actual.has(entry), true, `expected metadata to include ${entry}`)
  }
}


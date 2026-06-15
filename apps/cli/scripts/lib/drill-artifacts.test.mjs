import assert from "node:assert/strict"
import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
  DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS,
  DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
  DRILL_ARTIFACT_INDEX_SCHEMA,
  diagnosticMetadataForDrillArtifactIndexAggregate,
  findDrillArtifactIndexPaths,
  formatDrillArtifactIndexAggregateSummary,
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
  readDrillArtifactIndex,
  summarizeDrillArtifactIndexes,
  validateDrillArtifactIndexAggregate,
  validateDrillArtifactDiagnosticDimensions,
  validateDrillArtifactIndex,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
} from "./drill-artifacts.mjs"
import { drillRuntimeSignalsManifest } from "./drill-runtime-signals.mjs"

function validationSuiteRunArtifact(overrides = {}) {
  const manifest = overrides.manifest ?? validationSuiteManifestArtifact()
  return {
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:01.250Z",
    durationMs: 1250,
    exitCode: 0,
    signal: null,
    error: null,
    command: manifest.command,
    testCount: manifest.testCount,
    testPaths: manifest.testPaths,
    manifest,
    ...overrides,
  }
}

function validationSuiteManifestArtifact(overrides = {}) {
  return {
    schema: "arroba.drill.validation_suite.v1",
    command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
    testCount: 1,
    testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
    runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    ...overrides,
  }
}

function matrixReportArtifact(overrides = {}) {
  const scenarios = overrides.scenarios ?? [{
    id: "local",
    description: "local scenario",
    requires: [],
    exitCriteria: [],
    exitCriteriaEvidence: [],
    runtimeSignals: ["session-authority"],
    status: "passed",
    expectedFailure: false,
    classification: null,
    owner: null,
    nextAction: null,
    durationMs: 1,
    reason: null,
    command: "node",
    args: ["--version"],
    artifactHints: [],
  }]
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

function emptyDrillArtifactDiagnosticDimensions(overrides = {}) {
  return {
    ...Object.fromEntries(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS.map((key) => [key, {}])),
    ...overrides,
  }
}

test("drill artifacts are removed after a passing run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-pass-"))
  await prepareDrillArtifacts(root)

  const result = await finalizeDrillArtifacts({ rootDir: root, passed: true })

  assert.equal(result.preserved, false)
  await assert.rejects(stat(root), /ENOENT/)
})

test("drill artifacts are preserved with a failure manifest after a failed run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-fail-"))
  const events = []
  await prepareDrillArtifacts(root)

  const failure = new Error("relay target was stale with Bearer abcdefghijklmnopqrstuvwxyz")
  failure.stack = "Error: relay target was stale with sk-this-should-not-persist\n    at drill"

  const result = await finalizeDrillArtifacts({
    rootDir: root,
    passed: false,
    failure,
    metadata: {
      drill: "hosted-cloud-relay",
      relayToken: "relay-token-should-not-persist",
      provider: "Bearer abcdefghijklmnopqrstuvwxyz",
      nested: { apiKey: "sk-this-should-not-persist" },
    },
    log: (name, details) => events.push({ name, details }),
  })

  assert.equal(result.preserved, true)
  assert.equal(events[0].name, "preserved-failed-run")
  const manifest = JSON.parse(await readFile(result.manifestPath, "utf8"))
  assert.equal(manifest.schema, "arroba.drill.failure.v1")
  assert.equal(manifest.metadata.drill, "hosted-cloud-relay")
  assert.equal(manifest.metadata.relayToken, "<redacted>")
  assert.equal(manifest.metadata.provider, "<redacted>")
  assert.equal(manifest.metadata.nested.apiKey, "<redacted>")
  assert.equal(manifest.error.message, "relay target was stale with <redacted>")
  assert.match(manifest.error.stack, /Error: relay target was stale with <redacted>/)
  assert.doesNotMatch(JSON.stringify(manifest), /should-not-persist|abcdefghijklmnopqrstuvwxyz/)

  await finalizeDrillArtifacts({ rootDir: root, passed: true })
})

test("writes and verifies drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "gate.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
      status: "passed",
    })}\n`, "utf8")
    await writeFile(path.join(root, "reports", "notes.log"), "plain log\n", "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/notes.log", "reports/gate.json"],
      metadata: {
        drill: "validation-gate",
        relayToken: "relay-token-should-not-persist",
      },
    })
    const indexPath = path.join(root, "arroba-drill-artifacts.json")
    const readIndex = await readDrillArtifactIndex(indexPath)
    const verified = await verifyDrillArtifactIndex(indexPath)

    assert.equal(index.schema, DRILL_ARTIFACT_INDEX_SCHEMA)
    assert.deepEqual(readIndex, index)
    assert.deepEqual(verified, index)
    assert.equal(index.metadata.relayToken, "<redacted>")
    assert.deepEqual(index.artifacts.map((artifact) => artifact.path), [
      "reports/gate.json",
      "reports/notes.log",
    ])
    assert.deepEqual(index.artifacts.map((artifact) => artifact.schema), [
      "arroba.drill.validation_gate.v1",
      null,
    ])
    assert.doesNotMatch(JSON.stringify(index), /should-not-persist/)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("verifies runtime signal manifests embedded in validation artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    assert.deepEqual(await verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")), index)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts that advertise runtime signals without a manifest", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json is missing manifest\.runtimeSignalsManifest/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts with malformed runtime signal manifests", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    const manifest = drillRuntimeSignalsManifest()
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
        runtimeSignalsManifest: {
          ...manifest,
          signals: manifest.signals.filter((signal) => signal.id !== "lease-health"),
        },
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json\.manifest\.runtimeSignalsManifest does not match required runtime signals/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed validation suite run artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-run-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      ok: false,
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json ok does not match status/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite run artifacts with inconsistent manifest fields", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-run-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      testCount: 2,
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json\.testCount must match manifest\.testCount/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed validation suite manifest artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-manifest-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite.v1",
    }, null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite\.json is missing command/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite artifacts with mismatched metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-metadata-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify(validationSuiteManifestArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite.json"],
      metadata: {
        artifactKinds: "matrix-report",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite\.json metadata\.artifactKinds must include validation-suite/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite run artifacts with stale metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-metadata-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        artifactKinds: "validation-suite-run",
        status: "failed",
        tests: 1,
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json metadata\.status must match artifact status/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed matrix report artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-matrix-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "matrix.json"), `${JSON.stringify({
      schema: "arroba.drill.matrix.v1",
    }, null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/matrix.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /matrix\.json is missing matrix/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects matrix report artifacts with stale metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-matrix-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "matrix.json"), `${JSON.stringify(matrixReportArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/matrix.json"],
      metadata: {
        artifactKinds: "matrix-report",
        matrix: "other-matrix",
        status: "passed",
        dryRun: false,
        scenarios: 1,
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /matrix\.json metadata\.matrix must match artifact matrix/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects inconsistent drill runtime signal owner metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite.v1",
      runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    })}\n`, "utf8")

    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: { runtimeSignals: "session-authority" },
      }),
      /runtimeSignalOwners must match runtimeSignals/,
    )
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: { runtimeSignalOwners: "kernel-authority" },
      }),
      /runtimeSignalOwners requires runtimeSignals/,
    )
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: {
          runtimeSignals: "workspace-live-synch-state",
          runtimeSignalOwners: "runtime-state",
        },
      }),
      /drill runtime signals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("writes JSON artifact output with optional index", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-output-"))
  try {
    const outputPath = path.join(root, "reports", "gate.json")
    const artifactIndexPath = path.join(root, "reports", "arroba-drill-artifacts.json")
    const artifactIndex = await writeDrillJsonArtifactOutput({
      outputPath,
      artifactIndexPath,
      value: {
        schema: "arroba.drill.validation_gate.v1",
        status: "passed",
      },
      metadata: {
        drill: "validation-gate",
        token: "sk-this-should-not-persist",
      },
    })
    const fileValue = JSON.parse(await readFile(outputPath, "utf8"))
    const verified = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(fileValue.status, "passed")
    assert.deepEqual(verified, artifactIndex)
    assert.equal(artifactIndex.metadata.drill, "validation-gate")
    assert.equal(artifactIndex.metadata.token, "<redacted>")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "gate.json",
      schema: "arroba.drill.validation_gate.v1",
    }])
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("discovers drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "one", "reports"), { recursive: true })
    await mkdir(path.join(root, "two", "reports"), { recursive: true })
    await writeFile(path.join(root, "one", "reports", "gate.json"), "{\"schema\":\"one\"}\n", "utf8")
    await writeFile(path.join(root, "two", "reports", "gate.json"), "{\"schema\":\"two\"}\n", "utf8")
    const firstIndexPath = path.join(root, "one", "arroba-drill-artifacts.json")
    const secondIndexPath = path.join(root, "two", "arroba-drill-artifacts.json")
    await writeDrillArtifactIndex({
      rootDir: path.join(root, "one"),
      artifacts: ["reports/gate.json"],
    })
    await writeDrillArtifactIndex({
      rootDir: path.join(root, "two"),
      artifacts: ["reports/gate.json"],
    })
    await writeFile(path.join(root, "arroba-drill-artifacts.json"), "{\"schema\":\"other\"}\n", "utf8")

    assert.deepEqual(await findDrillArtifactIndexPaths([root]), [
      firstIndexPath,
      secondIndexPath,
    ])
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("summarizes drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "one", "reports"), { recursive: true })
    await mkdir(path.join(root, "two", "reports"), { recursive: true })
    await writeFile(path.join(root, "one", "reports", "gate.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
    })}\n`, "utf8")
    await writeFile(path.join(root, "one", "reports", "notes.log"), "plain log\n", "utf8")
    await writeFile(path.join(root, "two", "reports", "matrix.json"), `${JSON.stringify({
      schema: "arroba.drill.matrix.v1",
    })}\n`, "utf8")
    const first = await writeDrillArtifactIndex({
      rootDir: path.join(root, "one"),
      artifacts: ["reports/gate.json", "reports/notes.log"],
      metadata: {
        classifications: "validation-gate,artifact-coverage",
        owners: "validation-harness",
        runtimeSignals: "session-authority,provider-run-lifecycle",
        runtimeSignalOwners: "kernel-authority,provider-runtime",
        artifactKinds: "validation-gate,validation-suite-run",
        generatedEvidenceKinds: "validation-suite-run",
        generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
        requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
        missingGeneratedEvidenceKinds: "matrix-report",
        providerAccountAliases: "codex=work,opencode=zen",
        evidenceRepos: "oss",
      },
    })
    const second = await writeDrillArtifactIndex({
      rootDir: path.join(root, "two"),
      artifacts: ["reports/matrix.json"],
      metadata: {
        classifications: "matrix-coverage",
        owners: "validation-harness,runtime-network",
        exitCriterionStatuses: "dry-run",
        incompleteExitCriterionStatuses: "dry-run",
        runtimeSignals: "session-authority,lease-health",
        runtimeSignalOwners: "kernel-authority",
        artifactKinds: "matrix-report",
        generatedEvidenceKinds: "matrix-report",
        generatedMatrixLimitations: "dry-run-classification-coverage",
        requiredGeneratedEvidenceKinds: "matrix-report",
        requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
        missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
        providerAccountAliases: "codex=work,claude=team",
        evidenceRepos: "cloud,oss",
        artifactCoverageInputSources: "artifact metadata inputs",
      },
    })

    const aggregate = summarizeDrillArtifactIndexes([first, second], {
      sources: ["one/arroba-drill-artifacts.json", "two/arroba-drill-artifacts.json"],
    })

    assert.equal(aggregate.schema, DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA)
    assert.equal(aggregate.totals.indexes, 2)
    assert.equal(aggregate.totals.artifacts, 3)
    assert.deepEqual(aggregate.schemas, {
      "arroba.drill.matrix.v1": 1,
      "arroba.drill.validation_gate.v1": 1,
      none: 1,
    })
    assert.deepEqual(aggregate.runtimeSignals, {
      "lease-health": 1,
      "provider-run-lifecycle": 1,
      "session-authority": 2,
    })
    assert.deepEqual(aggregate.runtimeSignalOwners, {
      "kernel-authority": 2,
      "provider-runtime": 1,
    })
    assert.deepEqual(aggregate.owners, {
      "runtime-network": 1,
      "validation-harness": 2,
    })
    assert.deepEqual(aggregate.classifications, {
      "artifact-coverage": 1,
      "matrix-coverage": 1,
      "validation-gate": 1,
    })
    assert.deepEqual(aggregate.exitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(aggregate.incompleteExitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(aggregate.artifactKinds, {
      "matrix-report": 1,
      "validation-gate": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.generatedEvidenceKinds, {
      "matrix-report": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.generatedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.generatedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, {
      "matrix-report": 2,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, {
      "matrix-report": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.providerAccountAliases, {
      "claude=team": 1,
      "codex=work": 2,
      "opencode=zen": 1,
    })
    assert.deepEqual(aggregate.evidenceRepos, {
      cloud: 1,
      oss: 2,
    })
    assert.deepEqual(aggregate.artifactCoverageInputSources, {
      "artifact metadata inputs": 1,
    })
    assert.deepEqual(aggregate.indexes.map((index) => index.source), [
      "one/arroba-drill-artifacts.json",
      "two/arroba-drill-artifacts.json",
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.runtimeSignals), [
      {
        "provider-run-lifecycle": 1,
        "session-authority": 1,
      },
      {
        "lease-health": 1,
        "session-authority": 1,
      },
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.runtimeSignalOwners), [
      {
        "kernel-authority": 1,
        "provider-runtime": 1,
      },
      {
        "kernel-authority": 1,
      },
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.owners), [
      {
        "validation-harness": 1,
      },
      {
        "runtime-network": 1,
        "validation-harness": 1,
      },
    ])
    assert.deepEqual(diagnosticMetadataForDrillArtifactIndexAggregate(aggregate), {
      artifactKinds: "matrix-report,validation-gate,validation-suite-run",
      artifactCoverageInputCount: "1",
      artifactCoverageInputSources: "artifact metadata inputs",
      classifications: "artifact-coverage,matrix-coverage,validation-gate",
      exitCriterionStatuses: "dry-run",
      incompleteExitCriterionStatuses: "dry-run",
      evidenceRepos: "cloud,oss",
      generatedEvidenceKinds: "matrix-report,validation-suite-run",
      generatedMatrixLimitations: "dry-run-classification-coverage",
      generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
      missingGeneratedEvidenceKinds: "matrix-report",
      missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
      owners: "runtime-network,validation-harness",
      providerAccountAliases: "claude=team,codex=work,opencode=zen",
      requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
      requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
      runtimeSignalOwners: "kernel-authority,provider-runtime",
      runtimeSignals: "lease-health,provider-run-lifecycle,session-authority",
    })
    assert.deepEqual(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS, [
      "runtimeSignals",
      "runtimeSignalOwners",
      "coverageAreas",
      "owners",
      "classifications",
      "exitCriterionStatuses",
      "incompleteExitCriterionStatuses",
      "artifactKinds",
      "generatedEvidenceKinds",
      "generatedMatrixArtifactIndexes",
      "generatedMatrixLimitations",
      "generatedValidationSuiteFailureRoots",
      "requiredGeneratedEvidenceKinds",
      "missingGeneratedEvidenceKinds",
      "requiredGeneratedMatrixLimitations",
      "missingGeneratedMatrixLimitations",
      "providerAccountAliases",
      "evidenceRepos",
      "artifactCoverageInputSources",
    ])
    assert.deepEqual(DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS, [
      "schemas",
      ...DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
    ])
    for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
      assert(Object.hasOwn(aggregate, key), `aggregate should preserve ${key}`)
      assert(aggregate.indexes.every((index) => Object.hasOwn(index, key)), `index summaries should preserve ${key}`)
    }
    assert.doesNotThrow(() => validateDrillArtifactDiagnosticDimensions(aggregate))
    assert.doesNotThrow(() => validateDrillArtifactIndexAggregate(aggregate))
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /indexes=2 artifacts=3/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /runtime_signals: lease-health=1 provider-run-lifecycle=1 session-authority=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /runtime_signal_owners: kernel-authority=2 provider-runtime=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /owners: runtime-network=1 validation-harness=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /classifications: artifact-coverage=1 matrix-coverage=1 validation-gate=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /exit_criterion_statuses: dry-run=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /incomplete_exit_criterion_statuses: dry-run=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /provider_account_aliases: claude=team=1 codex=work=2 opencode=zen=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /artifact_coverage_input_count=1/)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

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
      owners: {},
      classifications: {},
      exitCriterionStatuses: {},
      incompleteExitCriterionStatuses: {},
      artifactKinds: {},
    }),
    /missing generatedEvidenceKinds/,
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
      generatedEvidenceKinds: { "matrix-reprot": 1 },
    })),
    /generatedEvidenceKinds has unknown generated evidence kind "matrix-reprot"/,
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
        rootDir: root,
        artifacts: ["../outside.json"],
      }),
      /escapes root/,
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

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
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite_run.v1",
      status: "passed",
      ok: true,
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        runtimeSignalsManifest: drillRuntimeSignalsManifest(),
      },
    }, null, 2)}\n`, "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    assert.deepEqual(await verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")), index)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts with malformed runtime signal manifests", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    const manifest = drillRuntimeSignalsManifest()
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite_run.v1",
      status: "passed",
      ok: true,
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        runtimeSignalsManifest: {
          ...manifest,
          signals: manifest.signals.filter((signal) => signal.id !== "lease-health"),
        },
      },
    }, null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json\.manifest\.runtimeSignalsManifest does not match required runtime signals/,
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
        artifactKinds: "validation-gate,validation-suite-run",
        generatedEvidenceKinds: "validation-suite-run",
        requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
        missingGeneratedEvidenceKinds: "matrix-report",
        evidenceRepos: "oss",
      },
    })
    const second = await writeDrillArtifactIndex({
      rootDir: path.join(root, "two"),
      artifacts: ["reports/matrix.json"],
      metadata: {
        classifications: "matrix-coverage",
        owners: "validation-harness,runtime-network",
        runtimeSignals: "session-authority,lease-health",
        artifactKinds: "matrix-report",
        generatedEvidenceKinds: "matrix-report",
        requiredGeneratedEvidenceKinds: "matrix-report",
        evidenceRepos: "cloud,oss",
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
    assert.deepEqual(aggregate.artifactKinds, {
      "matrix-report": 1,
      "validation-gate": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.generatedEvidenceKinds, {
      "matrix-report": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, {
      "matrix-report": 2,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, {
      "matrix-report": 1,
    })
    assert.deepEqual(aggregate.evidenceRepos, {
      cloud: 1,
      oss: 2,
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
      classifications: "artifact-coverage,matrix-coverage,validation-gate",
      evidenceRepos: "cloud,oss",
      generatedEvidenceKinds: "matrix-report,validation-suite-run",
      missingGeneratedEvidenceKinds: "matrix-report",
      owners: "runtime-network,validation-harness",
      requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
      runtimeSignalOwners: "kernel-authority,provider-runtime",
      runtimeSignals: "lease-health,provider-run-lifecycle,session-authority",
    })
    assert.deepEqual(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS, [
      "runtimeSignals",
      "runtimeSignalOwners",
      "coverageAreas",
      "owners",
      "classifications",
      "artifactKinds",
      "generatedEvidenceKinds",
      "requiredGeneratedEvidenceKinds",
      "missingGeneratedEvidenceKinds",
      "evidenceRepos",
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
        generatedEvidenceKinds: { "matrix-report": 1 },
      }),
      /generatedEvidenceKinds do not match indexes/,
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
      artifactKinds: {},
    }),
    /missing generatedEvidenceKinds/,
  )
  assert.throws(
    () => validateDrillArtifactDiagnosticDimensions({
      runtimeSignals: {},
      runtimeSignalOwners: { "kernel-authority": 1 },
      coverageAreas: {},
      owners: {},
      classifications: {},
      artifactKinds: {},
      generatedEvidenceKinds: {},
      requiredGeneratedEvidenceKinds: {},
      missingGeneratedEvidenceKinds: {},
      evidenceRepos: { oss: -1 },
    }),
    /evidenceRepos has invalid count/,
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

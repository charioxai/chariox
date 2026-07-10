import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"
import { drillRuntimeAuthorityManifest } from "./lib/drill-runtime-authority-invariants.mjs"
import { drillRuntimeSignalsManifest } from "./lib/drill-runtime-signals.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-cross-repo-validation-gate.mjs", import.meta.url))

export {
  assert,
  drillFailureTaxonomyManifest,
  drillRuntimeAuthorityManifest,
  drillRuntimeSignalsManifest,
  execFile,
  mkdir,
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
}

export async function writeMatrixReport(file, { matrix, metadata, scenarios }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }, null, 2)}\n`, "utf8")
}

export async function writeCloudFailureTaxonomyRegistry(cloudRoot, {
  classifications = drillFailureTaxonomyManifest().classifications
    .map((classification) => classification.kind === "docker-runtime"
      ? { ...classification, owner: "worker-kernel" }
      : classification),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudFailureTaxonomyManifest() {",
    `  return { schema: "arroba.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeValidationSuiteArtifact(rootDir, { metadata = {} } = {}) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    exitCode: 0,
    signal: null,
    error: null,
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    manifest: {
      schema: "arroba.drill.validation_suite.v1",
      testCount: 1,
      command: "node --test scripts/cloud-validation-suite.test.mjs",
      coverage: [{
        id: "suite-contract",
        description: "Cloud validation-suite contract",
        testCount: 1,
        testPaths: ["scripts/cloud-validation-suite.test.mjs"],
      }],
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    },
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      coverageAreas: "distributed-observability,suite-contract",
      ...metadata,
    },
  })
  return path.join(rootDir, "arroba-drill-artifacts.json")
}

export async function writeCloudGeneratedMatrixRegistry(cloudRoot, {
  matrices = [
    { name: "browser-terminal-resilience-matrix", repo: "cloud" },
    { name: "cloud-slice-runtime-matrix", repo: "cloud" },
    { name: "native-provider-tui-matrix", repo: "oss" },
    { name: "remote-agent-runtime-matrix", repo: "oss" },
    { name: "remote-home-extension-matrix", repo: "oss" },
    { name: "runtime-resilience-chaos-matrix", repo: "oss" },
    { name: "slice-runtime-matrix", repo: "oss" },
    { name: "workspace-live-sync-matrix", repo: "oss" },
  ],
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillGeneratedMatrixNamesManifest() {",
    `  return { schema: "arroba.cloud.drill.generated_matrix_names.v1", matrices: ${JSON.stringify(matrices)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeCloudRuntimeSignalsRegistry(cloudRoot, {
  signals = drillRuntimeSignalsManifest().signals,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeSignalsManifest() {",
    `  return { schema: "arroba.drill.runtime_signals.v1", signals: ${JSON.stringify(signals)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeCloudRuntimeAuthorityRegistry(cloudRoot, {
  invariants = drillRuntimeAuthorityManifest().invariants.map((invariant) => invariant.id === "relay-cloud-transport-only"
    ? { ...invariant, id: "cloud-control-plane-only", owner: "cloud-deployment" }
    : invariant),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-authority-invariants.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeAuthorityManifest() {",
    `  return { schema: "arroba.drill.runtime_authority_invariants.v1", invariants: ${JSON.stringify(invariants)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeFailureManifest(file, {
  drill = "failed-drill",
  message = "Token refresh failed: 401",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill },
    error: { name: "Error", message, stack: null },
  }, null, 2)}\n`, "utf8")
  return file
}

export function scenario(id, classification, runtimeSignals = [], overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    runtimeSignals,
    ...overrides,
  }
}

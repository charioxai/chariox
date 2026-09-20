import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { test } from "node:test"

import {
  REQUIRED_RESULT_FIELDS,
  compareSnapshots,
  createSignedSnapshot,
  makeFixtureResults,
} from "./managed-path1-provider-parity-drill.mjs"

function execChild(command, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, options)
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk })
    child.stderr.on("data", (chunk) => { stderr += chunk })
    child.once("error", reject)
    child.once("close", (code, signal) => resolve({
      code,
      signal,
      stdout,
      stderr,
    }))
  })
}

const SIGNING_KEY = Buffer.from("managed-path1-parity-fixture-key-20260920")
const REVIEWED_COMMIT = "c".repeat(40)
const BUILD_ID = "sha256:" + "d".repeat(64)
const DRIVER = path.join(
  path.dirname(new URL(import.meta.url).pathname),
  "managed-path1-provider-parity-drill.mjs",
)

function snapshot(topology, overrides = {}, metadata = {}) {
  return createSignedSnapshot({
    reviewedCommit: metadata.reviewedCommit ?? REVIEWED_COMMIT,
    buildId: metadata.buildId ?? BUILD_ID,
    topology,
    workerFresh: true,
    provider: {
      name: "codex",
      version: "fixture-provider-1",
      executable: "codex",
      official: true,
    },
    capture: {
      boundary: "official-provider-turn",
      runtimeProbe: true,
      invokedInsideProviderTurn: true,
      fixture: true,
    },
    results: makeFixtureResults(overrides),
    signingKey: SIGNING_KEY,
  })
}

function compare(ordinary, path1, options = {}) {
  return compareSnapshots(ordinary, path1, {
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
    allowFixture: true,
    ...options,
  })
}

test("green parity fixture compares every required runtime result", () => {
  const report = compare(snapshot("ordinary"), snapshot("path1"))
  assert.equal(report.ok, true, JSON.stringify(report))
  assert.equal(report.comparedFields, REQUIRED_RESULT_FIELDS.length)
})

test("tampered snapshot fixture fails signature verification", () => {
  const path1 = snapshot("path1")
  path1.results.exact_cwd.comparison.exact_match = false
  const report = compare(snapshot("ordinary"), path1)
  assert.equal(report.ok, false)
  assert.ok(report.errors.includes("path1:signature_invalid"))
})

test("missing-row fixture fails closed even when the remaining rows are signed", () => {
  const path1 = snapshot("path1")
  delete path1.results.shutdown_custom
  const report = compare(snapshot("ordinary"), path1)
  assert.equal(report.ok, false)
  assert.ok(report.errors.some((error) => error.includes("missing_result:shutdown_custom")))
})

test("topology and reviewed-head mismatch fixtures fail closed", () => {
  const wrongTopology = snapshot("ordinary")
  const topologyReport = compare(snapshot("ordinary"), wrongTopology)
  assert.equal(topologyReport.ok, false)
  assert.ok(topologyReport.errors.includes("path1:topology_mismatch"))

  const wrongHead = snapshot("path1", {}, { reviewedCommit: "e".repeat(40) })
  const headReport = compare(snapshot("ordinary"), wrongHead)
  assert.equal(headReport.ok, false)
  assert.ok(headReport.errors.includes("path1:reviewed_commit_mismatch"))
})

test("different-result fixture reports the exact parity row", () => {
  const path1 = snapshot("path1", {
    network_reachability: {
      reachable: false,
      endpoint: "parity-network",
    },
  })
  const report = compare(snapshot("ordinary"), path1)
  assert.equal(report.ok, false)
  assert.ok(report.errors.includes("different_result:network_reachability"))
})

test("cleanup-failure fixture is not accepted as parity", () => {
  const path1 = snapshot("path1", {
    cleanup: {
      status: "failed",
      comparison: {
        temporary_data_removed: false,
        control_state_separate: true,
        no_failure: false,
      },
      reason: "cleanup_failed",
    },
  })
  const report = compare(snapshot("ordinary"), path1)
  assert.equal(report.ok, false)
  assert.ok(report.errors.includes("path1:result_not_passed:cleanup"))
})

test("CLI emits only a machine-readable summary and exits nonzero on a tamper", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-path1-parity-fixtures-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const ordinaryPath = path.join(root, "ordinary.json")
  const path1Path = path.join(root, "path1.json")
  const reportPath = path.join(root, "report.json")
  await writeFile(ordinaryPath, JSON.stringify(snapshot("ordinary")) + "\n")
  await writeFile(path1Path, JSON.stringify(snapshot("path1")) + "\n")

  const environment = {
    ...process.env,
    CHARIOX_PARITY_SIGNING_KEY: "managed-path1-parity-fixture-key-20260920",
  }
  const green = await execChild(process.execPath, [
    DRIVER,
    "compare",
    "--ordinary", ordinaryPath,
    "--path1", path1Path,
    "--reviewed-commit", REVIEWED_COMMIT,
    "--build-id", BUILD_ID,
    "--report", reportPath,
    "--allow-fixture",
  ], { cwd: path.dirname(DRIVER), env: environment, stdio: ["ignore", "pipe", "pipe"] })
  assert.equal(green.code, 0, green.stderr)
  assert.deepEqual(JSON.parse(green.stdout), {
    schema: "chariox.managed-ordinary-path1-runtime-parity/v1",
    ok: true,
    compared_fields: REQUIRED_RESULT_FIELDS.length,
    errors: [],
    mode: "compare",
    report: reportPath,
  })
  assert.equal(JSON.parse(await readFile(reportPath, "utf8")).ok, true)

  const tampered = JSON.parse(await readFile(path1Path, "utf8"))
  tampered.results.cleanup.comparison.no_failure = false
  await writeFile(path1Path, JSON.stringify(tampered) + "\n")
  const red = await execChild(process.execPath, [
    DRIVER,
    "compare",
    "--ordinary", ordinaryPath,
    "--path1", path1Path,
    "--reviewed-commit", REVIEWED_COMMIT,
    "--build-id", BUILD_ID,
    "--allow-fixture",
  ], { cwd: path.dirname(DRIVER), env: environment, stdio: ["ignore", "pipe", "pipe"] })
  assert.notEqual(red.code, 0)
  assert.deepEqual(JSON.parse(red.stdout), {
    schema: "chariox.managed-ordinary-path1-runtime-parity/v1",
    ok: false,
    compared_fields: 0,
    errors: ["path1:signature_invalid"],
    mode: "compare",
    report: null,
  })
})

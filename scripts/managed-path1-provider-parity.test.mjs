import assert from "node:assert/strict"
import { test } from "node:test"

import {
  ALLOWED_CAPTURE_BOUNDARIES,
  MANDATORY_SHUTDOWN_TRIGGERS,
  REQUIRED_RESULT_FIELDS,
  compareSnapshots,
  createSignedSnapshot,
  makeFixtureResults,
  validateSnapshot,
} from "./managed-path1-provider-parity-drill.mjs"

const TEST_KEY = Buffer.from("managed-path1-parity-fixture-key-20260920")
const REVIEWED_COMMIT = "a".repeat(40)
const BUILD_ID = "sha256:" + "b".repeat(64)

function fixture(topology, overrides = {}) {
  return createSignedSnapshot({
    reviewedCommit: REVIEWED_COMMIT,
    buildId: BUILD_ID,
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
    signingKey: TEST_KEY,
  })
}

test("runtime parity contract enumerates every required row and shutdown trigger", () => {
  assert.ok(REQUIRED_RESULT_FIELDS.includes("exact_cwd"))
  assert.ok(REQUIRED_RESULT_FIELDS.includes("provider_ancestry"))
  assert.ok(REQUIRED_RESULT_FIELDS.includes("mount_visibility"))
  assert.ok(REQUIRED_RESULT_FIELDS.includes("cleanup"))
  assert.ok(REQUIRED_RESULT_FIELDS.includes("shutdown_agents_done"))
  assert.ok(REQUIRED_RESULT_FIELDS.includes("shutdown_deployment_reconciliation"))
  assert.deepEqual(MANDATORY_SHUTDOWN_TRIGGERS, [
    "agents_done",
    "idle_15m",
    "idle_30m",
    "minimum_3h",
    "manual",
    "custom",
    "explicit_lifecycle_reconciliation",
    "deployment_reconciliation",
  ])
  assert.deepEqual(ALLOWED_CAPTURE_BOUNDARIES, [
    "official-provider-turn",
    "remote-command",
  ])
})

test("source-only and incomplete snapshots cannot satisfy runtime acceptance", () => {
  const snapshot = fixture("ordinary")
  snapshot.capture.runtimeProbe = false
  const report = validateSnapshot(snapshot, {
    expectedTopology: "ordinary",
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: TEST_KEY,
    allowFixture: true,
  })
  assert.equal(report.ok, false)
  assert.ok(report.errors.includes("capture_not_runtime"))
})

test("comparator requires an explicit reviewed commit, build, and signed boundary", () => {
  const report = compareSnapshots(fixture("ordinary"), fixture("path1"), {
    expectedReviewedCommit: "",
    expectedBuildId: BUILD_ID,
    signingKey: TEST_KEY,
    allowFixture: true,
  })
  assert.equal(report.ok, false)
  assert.ok(report.errors.includes("expected_reviewed_commit_missing"))
})

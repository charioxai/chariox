import assert from "node:assert/strict"
import { test } from "node:test"

import {
  MATRIX_SCHEMA,
  ROW_DEFINITIONS,
  REPORT_SCHEMA,
  compareManifests,
  createParityMatrixRunner,
  createSignedManifest,
  runEvidenceCommand,
  serializeReport,
} from "./managed-ordinary-parity-matrix.mjs"

const SIGNING_KEY = Buffer.from("managed-ordinary-parity-matrix-fixture-key-20260920")
const REVIEWED_COMMIT = "c899b9ce9fe3ca91d49e771d9bd6bbde07416576"
const BUILD_ID = "build-kernel-20260920"
const KERNEL_BUILD_ID = "kernel-build-ordinary-managed-20260920"
const SOURCE_DIGEST = "sha256:" + "a".repeat(64)

function ordinaryResult(rowId, checkId) {
  if (rowId === "MP-04" && checkId === "provider_ancestry") {
    return { provider_observed: true, bwrap_ancestor: false, fresh_worker: true }
  }
  if (rowId === "MP-04" && checkId === "managed_isolation_environment") {
    return { managed_marker_absent: true, bwrap_environment_absent: true }
  }
  if (rowId === "MP-09") {
    return {
      exemption: "ordinary-not-applicable",
      observed: "ordinary-not-applicable",
      signed_release: false,
      signature_verified: false,
      atomic_activation: false,
      rollback_verified: false,
      release_digest: "not-applicable",
    }
  }
  if (rowId === "MP-10") {
    const trigger = checkId.slice("shutdown_".length)
    return {
      exemption: "ordinary-no-managed-shutdown",
      trigger,
      observed_outcome: "ordinary-no-managed-shutdown",
      managed_policy: false,
      shutdown_evidence: true,
      measured_from_last_agent_finished: trigger === "idle_15m" || trigger === "idle_30m",
    }
  }
  return {
    row: rowId,
    check: checkId,
    observed: true,
    value: `${rowId}/${checkId}`,
  }
}

function path1Result(rowId, checkId) {
  if (rowId === "MP-09") {
    return {
      exemption: "managed-signed-release",
      observed: "managed-signed-release",
      signed_release: true,
      signature_verified: true,
      atomic_activation: true,
      rollback_verified: true,
      release_digest: "sha256:" + "b".repeat(64),
    }
  }
  if (rowId === "MP-10") {
    const trigger = checkId.slice("shutdown_".length)
    return {
      exemption: "managed-auto-shutdown",
      trigger,
      observed_outcome: `managed-${trigger}-evidence`,
      managed_policy: true,
      shutdown_evidence: true,
      measured_from_last_agent_finished: trigger === "idle_15m" || trigger === "idle_30m",
    }
  }
  return ordinaryResult(rowId, checkId)
}

function makeManifest(topology) {
  const rows = {}
  for (const definition of ROW_DEFINITIONS) {
    const checks = {}
    for (const checkId of definition.checks) {
      checks[checkId] = {
        status: "pass",
        result: topology === "ordinary"
          ? ordinaryResult(definition.id, checkId)
          : path1Result(definition.id, checkId),
        command: `collector-${topology} ${definition.id} ${checkId}`,
        evidence_refs: [`evidence://${topology}/${definition.id}/${checkId}`],
      }
    }
    rows[definition.id] = { checks }
  }
  return createSignedManifest({
    schema: MATRIX_SCHEMA,
    manifest_kind: "ordinary-versus-managed-evidence",
    captured_at: "2026-09-20T00:00:00.000Z",
    identity: {
      reviewed_commit: REVIEWED_COMMIT,
      reviewed_build_id: BUILD_ID,
      kernel_build_id: KERNEL_BUILD_ID,
      source_digest: SOURCE_DIGEST,
      kernel_protocol: "kernel-protocol-v1",
      relay_protocol: "relay-protocol-v1",
    },
    topology,
    worker_fresh: true,
    provider: {
      name: "codex",
      version: "fixture-provider-1",
      executable: "codex",
      official: true,
    },
    collection: {
      boundary: "official-provider-turn",
      runtime_probe: true,
      inside_provider_turn: true,
      independent: true,
      fixture: true,
    },
    rows,
  }, SIGNING_KEY)
}

function cloneAndResign(manifest, mutate) {
  const copy = structuredClone(manifest)
  delete copy.signature
  mutate(copy)
  return createSignedManifest(copy, SIGNING_KEY)
}

function compare(ordinary = makeManifest("ordinary"), path1 = makeManifest("path1")) {
  return compareManifests(ordinary, path1, {
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
    allowFixture: true,
  })
}

test("green fixture covers MP-01 through MP-10 and permits only the two exemptions", () => {
  const report = compare()
  assert.equal(report.schema, REPORT_SCHEMA)
  assert.equal(report.status, "pass", JSON.stringify(report, null, 2))
  assert.deepEqual(report.rows.map((row) => row.id), ROW_DEFINITIONS.map((row) => row.id))
  assert.deepEqual(report.rows.filter((row) => row.exemption).map((row) => row.id), ["MP-09", "MP-10"])
  assert.equal(report.failures.length, 0)
})

test("missing row and missing shutdown check fail closed", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    delete manifest.rows["MP-04"]
    delete manifest.rows["MP-10"].checks.shutdown_custom
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => failure.code === "missing_row" && failure.rowId === "MP-04"))
  assert.ok(report.failures.some((failure) => failure.code === "missing_check" && failure.checkId === "shutdown_custom"))
})

test("stale reviewed commit and protocol identity are rejected", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.identity.reviewed_commit = "d".repeat(40)
    manifest.identity.relay_protocol = "relay-protocol-stale"
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => failure.code === "identity_mismatch" && failure.detail === "reviewed_commit"))
  assert.ok(report.failures.some((failure) => failure.code === "identity_mismatch" && failure.detail === "relay_protocol"))
})

test("hidden extra result differences are reported on their exact matrix check", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-02"].checks.exact_path_entry.result.hidden_extra_difference = "unexpected"
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "unapproved_difference"
      && failure.rowId === "MP-02"
      && failure.checkId === "exact_path_entry"
  )))
})

test("a Bubblewrap ancestor is a product-boundary failure even with matching ordinary evidence", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-04"].checks.provider_ancestry.result.bwrap_ancestor = true
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => failure.code === "bwrap_ancestor_present" && failure.topology === "path1"))
})

test("cwd mismatch is not an allowed managed difference", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-02"].checks.exact_path_entry.result.value = "/tmp/wrong-cwd"
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "unapproved_difference"
      && failure.rowId === "MP-02"
      && failure.checkId === "exact_path_entry"
  )))
})

test("shutdown evidence is mandatory and cannot be hidden behind the exemption", () => {
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-10"].checks.shutdown_idle_15m.result.shutdown_evidence = false
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "shutdown_evidence_missing"
      && failure.rowId === "MP-10"
      && failure.checkId === "shutdown_idle_15m"
  )))
})

test("filesystem and command seams are injectable without touching the host", async () => {
  const files = new Map([
    ["ordinary.json", JSON.stringify(makeManifest("ordinary"))],
    ["path1.json", JSON.stringify(makeManifest("path1"))],
  ])
  const writes = new Map()
  const calls = []
  const runner = createParityMatrixRunner({
    filesystem: {
      async readFile(filePath) { return files.get(filePath) },
      async writeFile(filePath, content) { writes.set(filePath, content) },
    },
    runCommand: async (command, args) => {
      calls.push([command, args])
      return { code: 0, signal: null, stdout: "fixture-output", stderr: "" }
    },
  })
  const report = await runner.compareFiles({
    ordinaryPath: "ordinary.json",
    path1Path: "path1.json",
    reportPath: "report.json",
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
    allowFixture: true,
  })
  assert.equal(report.status, "pass")
  assert.equal(JSON.parse(writes.get("report.json")).status, "pass")
  const commandResult = await runner.runEvidenceCommand("fixture-probe", ["--cwd", "/home"])
  assert.deepEqual(commandResult, {
    ok: true,
    code: 0,
    signal: null,
    stdout: "fixture-output",
    stderr: "",
  })
  assert.deepEqual(calls, [["fixture-probe", ["--cwd", "/home"]]])
})

test("command seam converts a failed probe into bounded evidence", async () => {
  const result = await runEvidenceCommand("fixture-failing-probe", [], {
    runCommand: async () => { throw Object.assign(new Error("probe failed"), { code: 17, signal: "SIGTERM" }) },
  })
  assert.deepEqual(result, {
    ok: false,
    code: 17,
    signal: "SIGTERM",
    stdout: "",
    stderr: "probe failed",
  })
})

test("serialized reports are deterministic and contain command/evidence references", () => {
  const report = compare()
  const first = serializeReport(report)
  const second = serializeReport(compare())
  assert.equal(first, second)
  assert.match(first, /"rowId": "MP-02"|"id": "exact_path_entry"/)
  assert.match(first, /collector-ordinary MP-02 exact_path_entry/)
  assert.match(first, /evidence:\/\/path1\/MP-10\/shutdown_custom/)
})

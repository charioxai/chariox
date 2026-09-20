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

const RESULT_TRUE_FIELDS = Object.freeze({
  "MP-02/directory_discovery": ["exact_path_accessible"],
  "MP-02/exact_path_entry": ["exact_path_accessible", "cwd_matches_requested"],
  "MP-02/directory_creation": ["created_and_accessible"],
  "MP-02/home_access": ["accessible"],
  "MP-02/tmp_access": ["accessible"],
  "MP-03/empty_workspace": ["workspace_created", "control_state_separate"],
  "MP-03/copied_repository": ["repository_accessible"],
  "MP-03/repository_basename": ["source_basename_preserved"],
  "MP-03/basename_collision": ["collision_rejected"],
  "MP-03/worktree_placement": ["worktree_user_path", "control_root_not_workspace"],
  "MP-04/provider_environment": ["home_matches_ordinary", "chariox_home_matches_ordinary", "cwd_matches_requested", "ordinary_user"],
  "MP-04/mount_visibility": ["mounts_match_ordinary", "mount_probe_complete"],
  "MP-04/privilege_state": ["capabilities_match_ordinary", "umask_matches_ordinary"],
  "MP-04/network_reachability": ["network_matches_ordinary", "address_families_recorded"],
  "MP-04/package_tool_installation": ["tool_probe_succeeded", "install_probe_succeeded"],
  "MP-05/session_agent_launch": ["session_created", "agent_created", "official_command"],
  "MP-05/terminal_file_git": ["terminal_ok", "file_ok", "git_ok"],
  "MP-05/attachments_permissions_capabilities": ["attachments_ok", "permissions_ok", "capabilities_ok"],
  "MP-05/project_setup": ["project_setup_ok"],
  "MP-06/reconnect_orphan_recovery": ["reconnect_ok", "orphan_recovered"],
  "MP-06/restart_recovery": ["restart_recovered"],
  "MP-06/reconnect_history_result_identity": ["history_preserved", "result_identity_preserved"],
  "MP-06/queued_prompts": ["queued_prompt_preserved", "queued_prompt_advanced"],
  "MP-06/active_turn_state": ["active_turn_state_preserved"],
  "MP-07/control_file_protection": ["control_file_denied", "sibling_accessible"],
  "MP-07/filesystem_permissions": ["permissions_match_ordinary"],
  "MP-07/resource_limits": ["limits_observed"],
  "MP-07/structured_errors": ["structured_errors"],
  "MP-07/protocol_behavior": ["protocol_behavior_ok"],
  "MP-08/cleanup": ["owned_processes_gone", "owned_artifacts_removed", "foreign_processes_untouched", "cleanup_complete"],
})

function ordinaryResult(rowId, checkId) {
  if (rowId === "MP-01" && checkId === "source_protocol_identity") {
    return {
      observed: true,
      source_sha_verified: true,
      source_clean: true,
      kernel_protocol: "kernel-protocol-v1",
      relay_protocol: "relay-protocol-v1",
      build_identity_verified: true,
    }
  }
  if (rowId === "MP-01" && checkId === "fresh_worker") {
    return { observed: true, fresh_worker: true, worker_identity_observed: true }
  }
  if (rowId === "MP-01" && checkId === "official_provider_identity") {
    return {
      observed: true,
      official: true,
      provider_name: "codex",
      executable_matches: true,
      executable_basename: "codex",
      provider_version_observed: true,
    }
  }
  if (rowId === "MP-01" && checkId === "capture_boundary") {
    return { observed: true, boundary_verified: true, inside_provider_turn: true, independent: true }
  }
  if (rowId === "MP-04" && checkId === "provider_ancestry") {
    return { observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true }
  }
  if (rowId === "MP-04" && checkId === "managed_isolation_environment") {
    return { observed: true, managed_marker_absent: true, bwrap_environment_absent: true }
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
  const fields = RESULT_TRUE_FIELDS[`${rowId}/${checkId}`]
  assert.ok(fields, `missing fixture result schema for ${rowId}/${checkId}`)
  const result = Object.fromEntries(["observed", ...fields].map((field) => [field, true]))
  if (rowId === "MP-02" && checkId === "directory_discovery") result.child_enumeration_denied = false
  if (rowId === "MP-04" && checkId === "privilege_state") result.no_new_privs = false
  return result
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
    failure.code === "check_result_shape_invalid"
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
    manifest.rows["MP-02"].checks.exact_path_entry.result.cwd_matches_requested = false
  })
  const report = compare(makeManifest("ordinary"), path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "check_result_semantics_invalid"
      && failure.rowId === "MP-02"
      && failure.checkId === "exact_path_entry"
  )))
})

test("matching placeholder objects cannot satisfy a typed result", () => {
  const ordinary = cloneAndResign(makeManifest("ordinary"), (manifest) => {
    manifest.rows["MP-05"].checks.terminal_file_git.result = { observed: true }
  })
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-05"].checks.terminal_file_git.result = { observed: true }
  })
  const report = compare(ordinary, path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "check_result_shape_invalid"
      && failure.rowId === "MP-05"
      && failure.checkId === "terminal_file_git"
  )))
})

test("matching false cleanup evidence fails closed", () => {
  const ordinary = cloneAndResign(makeManifest("ordinary"), (manifest) => {
    manifest.rows["MP-08"].checks.cleanup.result.cleanup_complete = false
  })
  const path1 = cloneAndResign(makeManifest("path1"), (manifest) => {
    manifest.rows["MP-08"].checks.cleanup.result.cleanup_complete = false
  })
  const report = compare(ordinary, path1)
  assert.equal(report.status, "fail")
  assert.ok(report.failures.some((failure) => (
    failure.code === "check_result_semantics_invalid"
      && failure.rowId === "MP-08"
      && failure.checkId === "cleanup"
      && failure.detail === "cleanup_complete"
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

test("comparison identity is mandatory before either manifest is read", async () => {
  const reads = []
  const runner = createParityMatrixRunner({
    filesystem: {
      async readFile(filePath) {
        reads.push(filePath)
        throw new Error("manifest read must not run")
      },
      async writeFile() {
        throw new Error("report write must not run")
      },
    },
  })
  await assert.rejects(
    runner.compareFiles({
      ordinaryPath: "ordinary.json",
      path1Path: "path1.json",
      expectedBuildId: BUILD_ID,
      signingKey: SIGNING_KEY,
    }),
    /expected reviewed commit/,
  )
  await assert.rejects(
    runner.compareFiles({
      ordinaryPath: "ordinary.json",
      path1Path: "path1.json",
      expectedReviewedCommit: REVIEWED_COMMIT,
      signingKey: SIGNING_KEY,
    }),
    /expected reviewed build id/,
  )
  assert.deepEqual(reads, [])
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

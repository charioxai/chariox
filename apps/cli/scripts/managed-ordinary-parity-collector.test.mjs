import assert from "node:assert/strict"
import { test } from "node:test"

import {
  CollectorError,
  createParityCollector,
} from "./managed-ordinary-parity-collector.mjs"
import {
  compareManifests,
  validateManifest,
} from "./managed-ordinary-parity-matrix.mjs"

const SIGNING_KEY = Buffer.from("managed-ordinary-parity-collector-fixture-key-20260920")
const REVIEWED_COMMIT = "d1e925f2b3e318b66160d65cac05973409c99b68"
const BUILD_ID = "chariox-kernel build-20260920"
const KERNEL_PROTOCOL = 332
const RELAY_PROTOCOL = 54

function memoryFilesystem() {
  const files = new Map()
  const directories = new Set()
  return {
    files,
    directories,
    async mkdir(directory) { directories.add(directory) },
    async writeFile(file, content) { files.set(file, String(content)) },
  }
}

function fixedClock() {
  let tick = 0
  return () => new Date(Date.UTC(2026, 8, 20, 12, 0, tick++))
}

function genericResult(rowId, checkId, topology) {
  if (rowId === "MP-01" && checkId === "fresh_worker") {
    return { observed: true, fresh_worker: true, worker_id: `${topology}-worker-1` }
  }
  if (rowId === "MP-01" && checkId === "official_provider_identity") {
    return { observed: true, official: true, provider_name: "codex", executable_matches: true }
  }
  if (rowId === "MP-01" && checkId === "capture_boundary") {
    return { observed: true, boundary: "official-provider-turn", inside_provider_turn: true, independent: true }
  }
  if (rowId === "MP-04" && checkId === "provider_ancestry") {
    return { observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true, ancestry_complete: true }
  }
  if (rowId === "MP-04" && checkId === "managed_isolation_environment") {
    return { observed: true, managed_marker_absent: true, bwrap_environment_absent: true }
  }
  if (rowId === "MP-04" && checkId === "privilege_state") {
    return { observed: true, no_new_privs: false, capabilities_match_ordinary: true, umask_matches_ordinary: true }
  }
  if (rowId === "MP-09") {
    return topology === "ordinary"
      ? {
          observed: true,
          ordinary_release_not_applicable: true,
          signed_release_present: false,
          signature_verified: false,
          atomic_activation: false,
          rollback_verified: false,
        }
      : {
          observed: true,
          signed_release_present: true,
          signature_verified: true,
          atomic_activation: true,
          rollback_verified: true,
          release_digest: "sha256:" + "b".repeat(64),
        }
  }
  if (rowId === "MP-10") {
    const trigger = checkId.slice("shutdown_".length)
    return {
      observed: true,
      trigger,
      observed_outcome: `${topology}-${trigger}-observed`,
      managed_policy: topology === "path1",
      shutdown_evidence: true,
      measured_from_last_agent_finished: trigger === "idle_15m" || trigger === "idle_30m",
    }
  }
  const flags = {
    "MP-02/directory_discovery": { exact_path_accessible: true, child_enumeration_denied: false },
    "MP-02/exact_path_entry": { exact_path_accessible: true, cwd_matches_requested: true },
    "MP-02/directory_creation": { created_and_accessible: true },
    "MP-02/home_access": { accessible: true },
    "MP-02/tmp_access": { accessible: true },
    "MP-03/empty_workspace": { workspace_created: true, control_state_separate: true },
    "MP-03/copied_repository": { repository_accessible: true },
    "MP-03/repository_basename": { source_basename_preserved: true },
    "MP-03/basename_collision": { collision_rejected: true },
    "MP-03/worktree_placement": { worktree_user_path: true, control_root_not_workspace: true },
    "MP-04/provider_environment": { home_matches_ordinary: true, chariox_home_matches_ordinary: true, cwd_matches_requested: true, ordinary_user: true },
    "MP-04/mount_visibility": { mounts_match_ordinary: true, mount_probe_complete: true },
    "MP-04/network_reachability": { network_matches_ordinary: true, address_families_recorded: true },
    "MP-04/package_tool_installation": { tool_probe_succeeded: true, install_probe_succeeded: true },
    "MP-05/session_agent_launch": { session_created: true, agent_created: true, official_command: true },
    "MP-05/terminal_file_git": { terminal_ok: true, file_ok: true, git_ok: true },
    "MP-05/attachments_permissions_capabilities": { attachments_ok: true, permissions_ok: true, capabilities_ok: true },
    "MP-05/project_setup": { project_setup_ok: true },
    "MP-06/reconnect_orphan_recovery": { reconnect_ok: true, orphan_recovered: true },
    "MP-06/restart_recovery": { restart_recovered: true },
    "MP-06/reconnect_history_result_identity": { history_preserved: true, result_identity_preserved: true },
    "MP-06/queued_prompts": { queued_prompt_preserved: true, queued_prompt_advanced: true },
    "MP-06/active_turn_state": { active_turn_state_preserved: true },
    "MP-07/control_file_protection": { control_file_denied: true, sibling_accessible: true },
    "MP-07/filesystem_permissions": { permissions_match_ordinary: true },
    "MP-07/resource_limits": { limits_observed: true },
    "MP-07/structured_errors": { structured_errors: true },
    "MP-07/protocol_behavior": { protocol_behavior_ok: true },
    "MP-08/cleanup": { owned_processes_gone: true, owned_artifacts_removed: true, foreign_processes_untouched: true, cleanup_complete: true },
  }
  return { observed: true, ...(flags[`${rowId}/${checkId}`] ?? {}) }
}

function makeHarness(topology, overrides = {}) {
  const filesystem = memoryFilesystem()
  const calls = []
  const clock = fixedClock()
  const defaultCommand = async (command, args) => {
    if (command === "git" && args[0] === "rev-parse") return { code: 0, stdout: REVIEWED_COMMIT + "\n", stderr: "" }
    if (command === "git" && args[0] === "diff") return { code: 0, stdout: "", stderr: "" }
    if (command === "git" && args[0] === "status") return { code: 0, stdout: "", stderr: "" }
    if (command === "git" && args[0] === "ls-files") return { code: 0, stdout: "100644 blob abc123\tapps/cli/package.json\n", stderr: "" }
    if (command === "git" && args[0] === "grep") return { code: 0, stdout: "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 54;\n", stderr: "" }
    if (command === "chariox-kernel" && args[0] === "--version") return { code: 0, stdout: BUILD_ID + "\n", stderr: "" }
    if (command === "chariox-kernel" && args[0] === "--print-local-daemon-protocol-version") return { code: 0, stdout: `${KERNEL_PROTOCOL}\n`, stderr: "" }
    if (command === "codex" && args[0] === "--version") return { code: 0, stdout: "codex fixture-20260920\n", stderr: "" }
    if (command === "parity-probe") {
      const rowIndex = args.indexOf("--parity-row")
      const checkIndex = args.indexOf("--parity-check")
      const rowId = rowIndex >= 0 ? args[rowIndex + 1] : "unknown"
      const check = checkIndex >= 0 ? args[checkIndex + 1] : "unknown"
      const result = overrides.results?.[`${rowId}/${check}`] ?? genericResult(rowId, check, topology)
      return { code: 0, stdout: JSON.stringify({ ok: true, result }), stderr: "" }
    }
    throw Object.assign(new Error(`unexpected command ${command}`), { code: "ENOENT" })
  }
  const runCommand = async (command, args) => {
    calls.push([command, [...args]])
    const overridden = overrides.command ? await overrides.command(command, args, defaultCommand) : undefined
    return overridden ?? defaultCommand(command, args)
  }
  const collector = createParityCollector({
    filesystem,
    runCommand,
    clock,
    processApi: { platform: "linux", pid: 77, cwd: () => "/repo" },
  })
  const options = {
    topology,
    reviewedCommit: REVIEWED_COMMIT,
    buildId: BUILD_ID,
    kernelProtocol: KERNEL_PROTOCOL,
    relayProtocol: RELAY_PROTOCOL,
    provider: "codex",
    providerCommand: "codex",
    kernelBinary: "chariox-kernel",
    probeCommand: "parity-probe",
    boundary: "official-provider-turn",
    outputPath: `/evidence/${topology}.json`,
    evidenceDir: `/evidence/${topology}-commands`,
    sourceRoot: "/repo",
    signingKey: SIGNING_KEY,
    timeoutMs: 100,
  }
  return { collector, options, filesystem, calls }
}

async function collect(topology, overrides = {}) {
  const harness = makeHarness(topology, overrides)
  const manifest = await harness.collector.collect(harness.options)
  return { ...harness, manifest }
}

test("collects a real-command ordinary snapshot and validates all required rows", async () => {
  const { manifest, filesystem, calls } = await collect("ordinary")
  assert.equal(manifest.topology, "ordinary")
  assert.equal(validateManifest(manifest, {
    expectedTopology: "ordinary",
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
  }).ok, true)
  assert.equal(calls.some(([command, args]) => command === "parity-probe" && args.includes("session_agent_launch")), true)
  assert.ok(filesystem.files.size > 30)
})

test("collects a fresh Path-1 managed snapshot and the comparator accepts ordinary-versus-managed parity", async () => {
  const ordinary = await collect("ordinary")
  const path1 = await collect("path1")
  const report = compareManifests(ordinary.manifest, path1.manifest, {
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
  })
  assert.equal(report.status, "pass", JSON.stringify(report, null, 2))
  assert.equal(path1.manifest.rows["MP-09"].checks.signed_release_activation.result.signed_release, true)
  assert.equal(path1.manifest.rows["MP-10"].checks.shutdown_idle_15m.result.managed_policy, true)
})

test("denied child enumeration still records a valid exact-path result", async () => {
  const { manifest } = await collect("ordinary", {
    results: {
      "MP-02/directory_discovery": {
        observed: true,
        exact_path_accessible: true,
        child_enumeration_denied: true,
      },
    },
  })
  const result = manifest.rows["MP-02"].checks.directory_discovery.result
  assert.deepEqual(result, { observed: true, exact_path_accessible: true, child_enumeration_denied: true })
})

test("missing command fails closed and retains a redacted evidence record", async () => {
  const harness = makeHarness("ordinary", {
    command(command, args) {
      if (command === "git" && args[0] === "rev-parse") throw Object.assign(new Error("missing git"), { code: "ENOENT" })
      throw new Error(`unexpected ${command}`)
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "command_not_found")
    return true
  })
  assert.ok([...harness.filesystem.files.values()].some((content) => content.includes("stdout_sha256")))
})

test("stale source identity is rejected even when every later caller value would look green", async () => {
  const harness = makeHarness("ordinary", {
    command(command, args) {
      if (command === "git" && args[0] === "rev-parse") return { code: 0, stdout: "a".repeat(40) + "\n", stderr: "" }
      throw new Error(`unexpected ${command}`)
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "source_identity_mismatch")
    return true
  })
})

test("Bubblewrap ancestry is a product-boundary failure", async () => {
  const harness = makeHarness("path1", {
    results: {
      "MP-04/provider_ancestry": {
        observed: true,
        provider_observed: true,
        bwrap_ancestor: true,
        fresh_worker: true,
        ancestry_complete: true,
      },
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "probe_assertion_failed")
    assert.equal(error.rowId, "MP-04")
    return true
  })
})

test("cwd mismatch cannot be normalized into a passing exact-path result", async () => {
  const harness = makeHarness("ordinary", {
    results: {
      "MP-02/exact_path_entry": { observed: true, exact_path_accessible: true, cwd_matches_requested: false },
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "cwd_mismatch")
    return true
  })
})

test("forged caller pass values and forged probe status are ignored", async () => {
  const harness = makeHarness("ordinary", {
    results: {
      "MP-02/home_access": { status: "pass", observed: false, accessible: true },
    },
  })
  harness.options.insideProviderTurn = true
  harness.options.forgedPass = true
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "probe_assertion_failed")
    return true
  })
})

test("malformed probe output fails closed", async () => {
  const harness = makeHarness("ordinary", {
    command(command, args) {
      if (command === "parity-probe" && args.includes("home_access")) {
        return { code: 0, stdout: "not-json\n", stderr: "" }
      }
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "malformed_output")
    return true
  })
})

test("command timeout fails closed", async () => {
  const harness = makeHarness("ordinary", {
    command(command, args) {
      if (command === "chariox-kernel" && args[0] === "--version") {
        throw Object.assign(new Error("probe timeout"), { code: "ETIMEDOUT", timedOut: true })
      }
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "command_timeout")
    return true
  })
})

test("partial cleanup is rejected instead of becoming an MP-08 pass", async () => {
  const harness = makeHarness("ordinary", {
    results: {
      "MP-08/cleanup": {
        observed: true,
        owned_processes_gone: true,
        owned_artifacts_removed: false,
        foreign_processes_untouched: true,
        cleanup_complete: false,
      },
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "probe_assertion_failed")
    assert.equal(error.rowId, "MP-08")
    return true
  })
})

test("evidence output is deterministic and redacts credentials", async () => {
  const first = makeHarness("ordinary", {
    results: {
      "MP-05/project_setup": { observed: true, project_setup_ok: true, token: "supersecret-token" },
    },
  })
  const second = makeHarness("ordinary", {
    results: {
      "MP-05/project_setup": { observed: true, project_setup_ok: true, token: "supersecret-token" },
    },
  })
  await first.collector.collect(first.options)
  await second.collector.collect(second.options)
  assert.deepEqual([...first.filesystem.files.entries()], [...second.filesystem.files.entries()])
  const allEvidence = [...first.filesystem.files.values()].join("\n")
  assert.equal(allEvidence.includes("supersecret-token"), false)
  assert.equal(allEvidence.includes("<redacted>"), true)
  assert.ok(allEvidence.includes("stdout_sha256"))
  assert.ok(allEvidence.includes("started_at"))
})

test("unsupported platform fails before any command can claim parity", async () => {
  const harness = makeHarness("ordinary")
  const collector = createParityCollector({
    filesystem: harness.filesystem,
    runCommand: async () => { throw new Error("must not execute") },
    clock: fixedClock(),
    processApi: { platform: "darwin", pid: 1, cwd: () => "/repo" },
  })
  await assert.rejects(() => collector.collect(harness.options), (error) => {
    assert.equal(error.code, "unsupported_platform")
    return true
  })
})

import assert from "node:assert/strict"
import { once } from "node:events"
import { createHash } from "node:crypto"
import { unlink } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, before, test } from "node:test"

import {
  createParityCollector,
  defaultRunCommand,
} from "./managed-ordinary-parity-collector.mjs"
import {
  CAPTURE_PROVENANCE_SCHEMA,
  SHUTDOWN_EXPECTATIONS,
  MATRIX_SCHEMA,
  compareManifests,
  validateManifest,
} from "./managed-ordinary-parity-matrix.mjs"
import {
  PROJECT_SETUP_PROOF_SCHEMA,
  projectSetupTargetIdentityDigest,
} from "./lib/managed-ordinary-project-setup-observer.mjs"

const SIGNING_KEY = Buffer.from("managed-ordinary-parity-collector-fixture-key-20260920")
const REVIEWED_COMMIT = "d1e925f2b3e318b66160d65cac05973409c99b68"
const BUILD_ID = "chariox-kernel build-20260920"
const KERNEL_PROTOCOL = 332
const RELAY_PROTOCOL = 54
const SOURCE_TREE = "e".repeat(40)
const PROBE_BLOB = "f".repeat(40)
const RELEASE_VERIFIER_BLOB = "1".repeat(40)
const KERNEL_BYTES = Buffer.from("signed-kernel-fixture-20260920\n")
const KERNEL_DIGEST = `sha256:${createHash("sha256").update(KERNEL_BYTES).digest("hex")}`
const RELEASE_DIGEST = `sha256:${"a".repeat(64)}`
const KERNEL_RELEASE_ROOT = "/release/rootfs"
const KERNEL_BINARY = `${KERNEL_RELEASE_ROOT}/usr/local/bin/chariox-kernel`
const KERNEL_RELEASE_PUBLIC_KEY = "/release/trusted-release-public-key"
const KERNEL_BUILDER_PUBLIC_KEY = "/release/trusted-builder-public-key"
const LIVE_KERNEL_PID = 43210
const LIVE_KERNEL_SOCKET_INODE = "880042"
const LIVE_KERNEL_SOCKET = join(tmpdir(), `chariox-managed-ordinary-parity-${process.pid}.sock`)
const LIVE_KERNEL_IDENTITY = Object.freeze({ kernel_id: "kernel-current", machine_id: "machine-current" })
const CAPTURE_PROVENANCE = Object.freeze({
  schema: CAPTURE_PROVENANCE_SCHEMA,
  boundary: "official-provider-turn",
  observed: true,
  kernel_identity: { ...LIVE_KERNEL_IDENTITY, transport: "local-unix-ipc" },
  session_id: "session-owned",
  agent_id: "agent-owned",
  attachment_id: "attachment-owned",
  prompt_id: "prompt-owned",
  prompt_origin: "chariox",
  prompt_status: "running",
  prompt_phase_start: "streaming",
  prompt_phase_end: "streaming",
  provider: {
    provider_run_id: "provider-run-owned",
    name: "codex",
    status: "Running",
    process_status: "active",
  },
  process: {
    pid: 201,
    linux_boot_id: "b4a8b0e7-0f5b-4fd8-bcd9-ccc1e8b3c5ac",
    start_time_ticks: "7001",
    ancestry_depth: 2,
    executable_basename: "codex",
    executable_path_sha256: `sha256:${"1".repeat(64)}`,
    executable_sha256: `sha256:${"2".repeat(64)}`,
    command_line_sha256: `sha256:${"3".repeat(64)}`,
    current_working_directory_sha256: `sha256:${"4".repeat(64)}`,
    launch_program_basename: "codex",
    launch_program_sha256: `sha256:${"5".repeat(64)}`,
    launch_arguments_sha256: `sha256:${"6".repeat(64)}`,
    launch_working_directory_sha256: `sha256:${"7".repeat(64)}`,
  },
})

function frame(payload) {
  const bytes = Buffer.from(JSON.stringify(payload), "utf8")
  const header = Buffer.allocUnsafe(4)
  header.writeUInt32BE(bytes.length, 0)
  return Buffer.concat([header, bytes])
}

let liveKernelServer

before(async () => {
  await unlink(LIVE_KERNEL_SOCKET).catch((error) => {
    if (error?.code !== "ENOENT") throw error
  })
  liveKernelServer = createServer((socket) => {
    let request = Buffer.alloc(0)
    socket.on("data", (chunk) => {
      request = Buffer.concat([request, chunk])
      if (request.length < 4) return
      const length = request.readUInt32BE(0)
      if (request.length < length + 4) return
      socket.end(frame({
        response: {
          RelayStatus: {
            status: { daemon_id: LIVE_KERNEL_IDENTITY.kernel_id, machine_id: LIVE_KERNEL_IDENTITY.machine_id },
          },
        },
        error: null,
      }))
    })
  })
  liveKernelServer.listen(LIVE_KERNEL_SOCKET)
  await once(liveKernelServer, "listening")
})

after(async () => {
  if (liveKernelServer?.listening) {
    await new Promise((resolve, reject) => liveKernelServer.close((error) => error ? reject(error) : resolve()))
  }
  await unlink(LIVE_KERNEL_SOCKET).catch((error) => {
    if (error?.code !== "ENOENT") throw error
  })
})

function procStat(pid, startTime = 123456) {
  const fields = ["S", "1", ...Array(17).fill("0"), String(startTime)]
  return `${pid} (chariox-kernel fixture) ${fields.join(" ")}\n`
}

// A production-shaped result fixture exercises the live observer schema. It
// models the probe response shape and does not claim that this test contacted a kernel.
function projectSetupFingerprint(value) {
  return `sha256:${createHash("sha256").update(String(value), "utf8").digest("hex")}`
}

function projectSetupProofFixture(overrides = {}, topology = "ordinary") {
  const suffix = topology === "ordinary" ? "ordinary" : "path1"
  const workerIdentity = projectSetupFingerprint(`${suffix}-worker`)
  const workerKernelIdentity = projectSetupFingerprint(`${suffix}-worker-kernel`)
  const platform = "linux-x86_64"
  const timestamp = 1_790_000_000_000 + (topology === "ordinary" ? 0 : 10_000)
  const snapshotDigest = projectSetupFingerprint(`${suffix}-snapshot`)
  return {
    schema: PROJECT_SETUP_PROOF_SCHEMA,
    product_api_observation: "kernel-public-api",
    ready_validation_verified: true,
    status_fresh: true,
    before_after_identity_stable: true,
    home_kernel_identity_fingerprint: projectSetupFingerprint(`${LIVE_KERNEL_IDENTITY.kernel_id}\0${LIVE_KERNEL_IDENTITY.machine_id}`),
    session_identity_fingerprint: projectSetupFingerprint(CAPTURE_PROVENANCE.session_id),
    agent_identity_fingerprint: projectSetupFingerprint(CAPTURE_PROVENANCE.agent_id),
    project_identity_fingerprint: projectSetupFingerprint(`${suffix}-project`),
    operation_identity_fingerprint: projectSetupFingerprint(`${suffix}-operation`),
    operation_attempt: topology === "ordinary" ? 1 : 2,
    operation_created_at_ms: timestamp,
    status_updated_at_ms: timestamp + 500,
    observation_started_at_ms: timestamp + 600,
    observation_finished_at_ms: timestamp + 700,
    worker_identity_fingerprint: workerIdentity,
    worker_kernel_identity_fingerprint: workerKernelIdentity,
    target_identity_digest: projectSetupTargetIdentityDigest(workerIdentity, workerKernelIdentity, platform),
    platform,
    definition_digest: `sha256:${"8".repeat(64)}`,
    definition_origin: "utility_generated",
    definition_source: "commands",
    definition_identity_verified: true,
    validation_command_count: 1,
    validation_receipts: [{
      command_digest: `sha256:${"9".repeat(64)}`,
      exit_code: 0,
      stdout_bytes: topology === "ordinary" ? 1 : 19,
      stderr_bytes: topology === "ordinary" ? 0 : 3,
    }],
    before_snapshot_digest: snapshotDigest,
    after_snapshot_digest: snapshotDigest,
    transport_kind: topology === "ordinary" ? "relay" : "kernel-public-api",
    endpoint_fingerprint: projectSetupFingerprint(`${suffix}-endpoint`),
    ...overrides,
  }
}

function memoryFilesystem() {
  const files = new Map()
  const directories = new Set(["/repo"])
  directories.add(`/proc/${LIVE_KERNEL_PID}/fd`)
  return {
    files,
    directories,
    async mkdir(directory) { directories.add(directory) },
    async readFile(file) {
      if (file === "/proc/net/unix") {
        return `Num RefCount Protocol Flags Type St Inode Path\n0000000000000000: 00000002 00000000 00010000 0001 01 ${LIVE_KERNEL_SOCKET_INODE} ${LIVE_KERNEL_SOCKET}\n`
      }
      if (file === "/proc/sys/kernel/random/boot_id") return "7a2b9ea4-cd3d-4bc2-9b36-46cba658217a\n"
      if (file === `/proc/${LIVE_KERNEL_PID}/stat`) return procStat(LIVE_KERNEL_PID)
      if (file === `/proc/${LIVE_KERNEL_PID}/exe`) return KERNEL_BYTES
      if (!files.has(file)) throw Object.assign(new Error(`missing fixture file ${file}`), { code: "ENOENT" })
      return files.get(file)
    },
    async readdir(directory) {
      if (directory === "/proc") return [String(LIVE_KERNEL_PID)]
      if (directory === `/proc/${LIVE_KERNEL_PID}/fd`) return ["3"]
      throw Object.assign(new Error(`missing fixture directory ${directory}`), { code: "ENOENT" })
    },
    async readlink(file) {
      if (file === `/proc/${LIVE_KERNEL_PID}/exe`) return KERNEL_BINARY
      if (file === `/proc/${LIVE_KERNEL_PID}/fd/3`) return `socket:[${LIVE_KERNEL_SOCKET_INODE}]`
      throw Object.assign(new Error(`missing fixture link ${file}`), { code: "ENOENT" })
    },
    async realpath(file) {
      if (directories.has(file)) return file
      if (file !== KERNEL_BINARY && file !== `${KERNEL_RELEASE_ROOT}/usr/local/bin/chariox-kernel`) {
        throw Object.assign(new Error(`missing fixture path ${file}`), { code: "ENOENT" })
      }
      return KERNEL_BINARY
    },
    async writeFile(file, content) { files.set(file, String(content)) },
  }
}

function fixedClock() {
  let tick = 0
  return () => new Date(Date.UTC(2026, 8, 20, 12, 0, tick++))
}

function genericResult(rowId, checkId, topology) {
  if (rowId === "MP-10" && checkId === "fresh_worker") {
    return { observed: true, fresh_worker: true, worker_id: `${topology}-worker-1` }
  }
  if (rowId === "MP-08" && checkId === "official_provider_identity") {
    return { observed: true, official: true, provider_name: "codex", executable_matches: true }
  }
  if (rowId === "MP-10" && checkId === "capture_boundary") {
    return {
      observed: true,
      boundary: "official-provider-turn",
      inside_provider_turn: true,
      independent: true,
      capture_provenance: CAPTURE_PROVENANCE,
    }
  }
  if (rowId === "MP-08" && checkId === "project_setup") {
    return {
      observed: true,
      project_setup_ok: true,
      project_setup_proof: projectSetupProofFixture({}, topology),
    }
  }
  if (rowId === "MP-01" && checkId === "provider_ancestry") {
    return { observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true, ancestry_complete: true }
  }
  if (rowId === "MP-01" && checkId === "managed_isolation_environment") {
    return { observed: true, managed_marker_absent: true, bwrap_environment_absent: true }
  }
  if (rowId === "MP-01" && checkId === "privilege_state") {
    return { observed: true, no_new_privs: false, capabilities_match_ordinary: true, umask_matches_ordinary: true }
  }
  if (rowId === "MP-07") {
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
  if (rowId === "MP-09") {
    const trigger = checkId.slice("shutdown_".length)
    const expectation = SHUTDOWN_EXPECTATIONS[trigger]
    const configuredDelay = expectation.delay === "positive"
      ? (trigger === "agents_done" ? 900 : 600)
      : expectation.delay
    const ordinary = topology === "ordinary"
    const expectedOutcome = ordinary ? "ordinary-remained-running" : expectation.outcome
    return {
      observed: true,
      trigger,
      expected_outcome: expectedOutcome,
      observed_outcome: expectedOutcome,
      managed_policy: !ordinary,
      observation_complete: true,
      worker_stopped: ordinary ? false : expectation.workerStopped,
      cleanup_confirmed: ordinary ? false : expectation.cleanupConfirmed,
      measured_from_last_agent_finished: ordinary ? false : expectation.measuredFromLastAgentFinished,
      configured_delay_seconds: ordinary ? null : configuredDelay,
      observed_delay_seconds: ordinary || !expectation.observesDelay ? null : configuredDelay,
    }
  }
  const flags = {
    "MP-01/mount_visibility": { mounts_match_ordinary: true, mount_probe_complete: true },
    "MP-01/network_reachability": { network_matches_ordinary: true, address_families_recorded: true },
    "MP-01/package_tool_installation": { tool_probe_succeeded: true, install_probe_succeeded: true },
    "MP-02/directory_discovery": { exact_path_accessible: true, child_enumeration_denied: false },
    "MP-02/exact_path_entry": { exact_path_accessible: true, cwd_matches_requested: true },
    "MP-02/directory_creation": { created_and_accessible: true },
    "MP-02/home_access": { accessible: true },
    "MP-02/tmp_access": { accessible: true },
    "MP-03/control_file_protection": { control_file_denied: true, parent_workspace_accessible: true, sibling_accessible: true },
    "MP-03/filesystem_permissions": { permissions_match_ordinary: true },
    "MP-04/provider_environment": { home_matches_ordinary: true, chariox_home_matches_ordinary: true, cwd_matches_requested: true, ordinary_user: true },
    "MP-05/empty_workspace": { workspace_created: true, control_state_separate: true },
    "MP-05/copied_repository": { repository_accessible: true },
    "MP-05/repository_basename": { source_basename_preserved: true },
    "MP-05/basename_collision": { collision_rejected: true },
    "MP-05/worktree_placement": { worktree_user_path: true, control_root_not_workspace: true },
    "MP-06/repository_root_default": { default_root_correct: true },
    "MP-06/repository_root_custom": { custom_root_persisted: true },
    "MP-06/repository_root_inheritance": { child_inherits_root: true },
    "MP-06/repository_root_override_rejected": { client_override_rejected: true },
    "MP-08/session_agent_launch": { session_created: true, agent_created: true, official_command: true },
    "MP-08/terminal_file_git": { terminal_ok: true, file_ok: true, git_ok: true },
    "MP-08/attachments_permissions_capabilities": { attachments_ok: true, permissions_ok: true, capabilities_ok: true },
    "MP-08/reconnect_orphan_recovery": { reconnect_ok: true, orphan_recovered: true },
    "MP-08/restart_recovery": { restart_recovered: true },
    "MP-08/reconnect_history_result_identity": { history_preserved: true, result_identity_preserved: true },
    "MP-08/queued_prompts": { queued_prompt_preserved: true, queued_prompt_advanced: true },
    "MP-08/active_turn_state": { active_turn_state_preserved: true },
    "MP-08/resource_limits": { limits_observed: true },
    "MP-08/structured_errors": { structured_errors: true },
    "MP-08/protocol_behavior": { protocol_behavior_ok: true },
    "MP-08/cleanup": { owned_processes_gone: true, owned_artifacts_removed: true, foreign_processes_untouched: true, cleanup_complete: true },
  }
  return { observed: true, ...(flags[`${rowId}/${checkId}`] ?? {}) }
}

function makeHarness(topology, overrides = {}) {
  const filesystem = memoryFilesystem()
  const processCwd = overrides.processCwd ?? "/repo"
  filesystem.directories.add(processCwd)
  filesystem.files.set(`${KERNEL_RELEASE_ROOT}/usr/lib/chariox/release-manifest.json`, JSON.stringify({
    schemaVersion: 2,
    sourceCommit: REVIEWED_COMMIT,
    sourceTree: SOURCE_TREE,
    artifacts: [{ name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: KERNEL_DIGEST }],
    ...overrides.releaseManifest,
  }))
  filesystem.files.set(KERNEL_BINARY, KERNEL_BYTES)
  const calls = []
  const clock = fixedClock()
  const defaultCommand = async (command, args) => {
    if (command === "git" && args[0] === "rev-parse" && args[1] === `${REVIEWED_COMMIT}^{tree}`) return { code: 0, stdout: SOURCE_TREE + "\n", stderr: "" }
    if (command === "git" && args[0] === "rev-parse") return { code: 0, stdout: REVIEWED_COMMIT + "\n", stderr: "" }
    if (command === "git" && args[0] === "diff") return { code: 0, stdout: "", stderr: "" }
    if (command === "git" && args[0] === "status") return { code: 0, stdout: "", stderr: "" }
    if (command === "git" && args[0] === "ls-files") return { code: 0, stdout: "100644 blob abc123\tapps/cli/package.json\n", stderr: "" }
    if (command === "git" && args[0] === "ls-tree" && args.at(-1) === "apps/cli/scripts/managed-ordinary-parity-probe.mjs") return { code: 0, stdout: `100644 blob ${PROBE_BLOB}\tapps/cli/scripts/managed-ordinary-parity-probe.mjs\n`, stderr: "" }
    if (command === "git" && args[0] === "hash-object" && args.at(-1) === "apps/cli/scripts/managed-ordinary-parity-probe.mjs") return { code: 0, stdout: `${PROBE_BLOB}\n`, stderr: "" }
    if (command === "git" && args[0] === "ls-tree" && args.at(-1) === "deploy/managed-kernel/verify-image-release.mjs") return { code: 0, stdout: `100644 blob ${RELEASE_VERIFIER_BLOB}\tdeploy/managed-kernel/verify-image-release.mjs\n`, stderr: "" }
    if (command === "git" && args[0] === "hash-object" && args.at(-1) === "deploy/managed-kernel/verify-image-release.mjs") return { code: 0, stdout: `${RELEASE_VERIFIER_BLOB}\n`, stderr: "" }
    if (command === "git" && args[0] === "grep") return { code: 0, stdout: "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 54;\n", stderr: "" }
    if (command === process.execPath && args[0].endsWith("verify-image-release.mjs")) return { code: 0, stdout: "", stderr: "" }
    if (command === KERNEL_BINARY && args[0] === "--version") return { code: 0, stdout: BUILD_ID + "\n", stderr: "" }
    if (command === KERNEL_BINARY && args[0] === "--print-local-daemon-protocol-version") return { code: 0, stdout: `${KERNEL_PROTOCOL}\n`, stderr: "" }
    if (command === "codex" && args[0] === "--version") return { code: 0, stdout: "codex fixture-20260920\n", stderr: "" }
    if (command === process.execPath && args[0].endsWith("managed-ordinary-parity-probe.mjs")) {
      const rowIndex = args.indexOf("--parity-row")
      const checkIndex = args.indexOf("--parity-check")
      const rowId = rowIndex >= 0 ? args[rowIndex + 1] : "unknown"
      const check = checkIndex >= 0 ? args[checkIndex + 1] : "unknown"
      const result = overrides.results?.[`${rowId}/${check}`] ?? genericResult(rowId, check, topology)
      return { code: 0, stdout: JSON.stringify({ ok: true, result: {
        probe_identity_verified: true,
        probe_source_commit: REVIEWED_COMMIT,
        probe_file: "apps/cli/scripts/managed-ordinary-parity-probe.mjs",
        probe_file_git_blob: PROBE_BLOB,
        ...result,
      } }), stderr: "" }
    }
    throw Object.assign(new Error(`unexpected command ${command}`), { code: "ENOENT" })
  }
  const runCommand = async (command, args, options = {}) => {
    calls.push([command, [...args], options])
    const overridden = overrides.command ? await overrides.command(command, args, defaultCommand) : undefined
    return overridden ?? defaultCommand(command, args)
  }
  const providerTurnBindingFactory = overrides.providerTurnBindingFactory ?? (async ({ expectedProvider, expectedBoundary }) => {
    assert.equal(expectedProvider, "codex")
    assert.equal(expectedBoundary, "official-provider-turn")
    return {
      socketPath: LIVE_KERNEL_SOCKET,
      kernelIdentity: { ...LIVE_KERNEL_IDENTITY, transport: "local-unix-ipc" },
      initialProof: CAPTURE_PROVENANCE,
      async finish() { return CAPTURE_PROVENANCE },
    }
  })
  const collector = createParityCollector({
    filesystem,
    runCommand,
    clock,
    providerTurnBindingFactory,
    processApi: {
      platform: "linux",
      pid: 77,
      cwd: () => processCwd,
      env: {
        CHARIOX_DAEMON_SOCKET: LIVE_KERNEL_SOCKET,
        CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({
          observed: false,
          boundary: "remote-command",
          inside_provider_turn: false,
          independent: false,
          kernel_identity: {
            observed: false,
            kernel_id: "caller-stale-kernel",
            machine_id: "caller-stale-machine",
            transport: "remote",
          },
        }),
      },
    },
  })
  const options = {
    topology,
    reviewedCommit: REVIEWED_COMMIT,
    buildId: BUILD_ID,
    kernelProtocol: KERNEL_PROTOCOL,
    relayProtocol: RELAY_PROTOCOL,
    provider: "codex",
    providerCommand: "codex",
    kernelBinary: KERNEL_BINARY,
    kernelReleaseRoot: KERNEL_RELEASE_ROOT,
    kernelReleaseDigest: RELEASE_DIGEST,
    kernelReleasePublicKey: KERNEL_RELEASE_PUBLIC_KEY,
    kernelBuilderPublicKey: topology === "path1" ? KERNEL_BUILDER_PUBLIC_KEY : undefined,
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
  assert.equal(calls.some(([command, args]) => command === process.execPath && args[0].endsWith("managed-ordinary-parity-probe.mjs") && args.includes("session_agent_launch")), true)
  const sourceIdentity = manifest.rows["MP-10"].checks.source_protocol_identity
  const bindingPath = sourceIdentity.evidence_refs.find((reference) => reference.endsWith("live-kernel-binding.json"))
  assert.ok(bindingPath)
  const binding = JSON.parse(filesystem.files.get(bindingPath))
  assert.equal(binding.schema, "chariox.managed-ordinary-live-kernel-binding/v1")
  assert.equal(binding.observed, true)
  assert.equal(binding.observed_kernel.kernel_id, LIVE_KERNEL_IDENTITY.kernel_id)
  assert.equal(binding.transport.kind, "local-unix-ipc")
  assert.equal(binding.kernel_process.executable_sha256, KERNEL_DIGEST)
  assert.equal(binding.stable_across_capture, true)
  assert.ok(filesystem.files.size > 30)
})

test("MP-08 production-shaped Project setup proof fixture roundtrips through collection and matrix validation", async () => {
  const manifests = {}
  for (const topology of ["ordinary", "path1"]) {
    const { manifest } = await collect(topology)
    manifests[topology] = manifest
    const check = manifest.rows["MP-08"].checks.project_setup
    assert.equal(manifest.schema, MATRIX_SCHEMA)
    assert.equal(check.status, "pass")
    assert.equal(check.result.project_setup_ok, true)
    assert.equal(check.result.project_setup_proof.schema, PROJECT_SETUP_PROOF_SCHEMA)
    assert.equal(check.result.project_setup_proof.product_api_observation, "kernel-public-api")
    assert.equal(validateManifest(manifest, {
      expectedTopology: topology,
      expectedReviewedCommit: REVIEWED_COMMIT,
      expectedBuildId: BUILD_ID,
      signingKey: SIGNING_KEY,
    }).ok, true)
  }
  const ordinaryProof = manifests.ordinary.rows["MP-08"].checks.project_setup.result.project_setup_proof
  const path1Proof = manifests.path1.rows["MP-08"].checks.project_setup.result.project_setup_proof
  assert.notEqual(ordinaryProof.operation_identity_fingerprint, path1Proof.operation_identity_fingerprint)
  assert.notEqual(ordinaryProof.endpoint_fingerprint, path1Proof.endpoint_fingerprint)
  assert.notEqual(ordinaryProof.before_snapshot_digest, path1Proof.before_snapshot_digest)
  assert.notEqual(ordinaryProof.validation_receipts[0].stdout_bytes, path1Proof.validation_receipts[0].stdout_bytes)
  const report = compareManifests(manifests.ordinary, manifests.path1, {
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
  })
  assert.equal(report.status, "pass", JSON.stringify(report, null, 2))
})

test("collector rejects injected or legacy Project setup assertions", async (context) => {
  const variants = [
    {
      name: "injected observer transport",
      result: {
        observed: true,
        project_setup_ok: true,
        project_setup_proof: projectSetupProofFixture({ product_api_observation: "injected-test-transport" }),
      },
    },
    {
      name: "legacy boolean only",
      result: { observed: true, project_setup_ok: true },
    },
    {
      name: "mismatched target digest",
      result: {
        observed: true,
        project_setup_ok: true,
        project_setup_proof: projectSetupProofFixture({ target_identity_digest: `sha256:${"c".repeat(64)}` }),
      },
      errorCode: "project_setup_proof_invalid",
    },
    {
      name: "proof identity belongs to a different provider-turn capture",
      result: {
        observed: true,
        project_setup_ok: true,
        project_setup_proof: projectSetupProofFixture({
          session_identity_fingerprint: projectSetupFingerprint("different-session"),
        }),
      },
      errorCode: "project_setup_capture_identity_mismatch",
    },
  ]
  for (const variant of variants) {
    await context.test(variant.name, async () => {
      const harness = makeHarness("ordinary", {
        results: { "MP-08/project_setup": variant.result },
      })
      await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
        assert.equal(error.code, variant.errorCode ?? "project_setup_proof_invalid")
        assert.equal(error.rowId, "MP-08")
        assert.equal(error.checkId, "project_setup")
        return true
      })
    })
  }
})

test("collects schema-3 native release identities for ordinary and Path-1 parity", async () => {
  const releaseManifest = { schemaVersion: 3, managedUpdateEvidenceVersion: 1 }
  const ordinary = await collect("ordinary", { releaseManifest })
  const path1 = await collect("path1", { releaseManifest })
  const report = compareManifests(ordinary.manifest, path1.manifest, {
    expectedReviewedCommit: REVIEWED_COMMIT,
    expectedBuildId: BUILD_ID,
    signingKey: SIGNING_KEY,
  })
  assert.equal(report.status, "pass", JSON.stringify(report, null, 2))
})

test("rejects unsupported schema-3 capabilities and mixed legacy parity identities", async (t) => {
  for (const [name, releaseManifest] of [
    ["missing capability", { schemaVersion: 3 }],
    ["capability zero", { schemaVersion: 3, managedUpdateEvidenceVersion: 0 }],
    ["future capability", { schemaVersion: 3, managedUpdateEvidenceVersion: 2 }],
    ["string capability", { schemaVersion: 3, managedUpdateEvidenceVersion: "1" }],
    ["null capability", { schemaVersion: 3, managedUpdateEvidenceVersion: null }],
    ["legacy capability claim", { schemaVersion: 2, managedUpdateEvidenceVersion: 1 }],
    ["future schema", { schemaVersion: 4, managedUpdateEvidenceVersion: 1 }],
  ]) {
    await t.test(name, async () => {
      const harness = makeHarness("ordinary", { releaseManifest })
      await assert.rejects(harness.collector.collect(harness.options), (error) => error.code === "kernel_release_identity_mismatch")
    })
  }
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
  assert.equal(path1.manifest.rows["MP-07"].checks.signed_release_activation.result.signed_release, true)
  assert.equal(path1.manifest.rows["MP-09"].checks.shutdown_idle_15m.result.managed_policy, true)
  const releaseVerification = path1.calls.find(([command, args]) =>
    command === process.execPath && args[0].endsWith("verify-image-release.mjs"),
  )
  assert.deepEqual(releaseVerification[1].slice(-2), ["path1", KERNEL_BUILDER_PUBLIC_KEY])
})

test("Path-1 capture requires an external builder trust root", async () => {
  const harness = makeHarness("path1")
  delete harness.options.kernelBuilderPublicKey
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "builder_key_missing")
    return true
  })
})

test("exact-path collector capture binds the target to its invocation cwd and probe cwd", async () => {
  const target = "/tmp/chariox-parity-target/workspace"
  const harness = makeHarness("ordinary", { processCwd: target })
  harness.options.expectedCwd = target
  const manifest = await harness.collector.collect(harness.options)
  const [command, args, options] = harness.calls.find(([command, args]) =>
    command === process.execPath
      && args[0].endsWith("managed-ordinary-parity-probe.mjs")
      && args.includes("exact_path_entry"),
  )
  assert.equal(command, process.execPath)
  assert.equal(args[args.indexOf("--expected-cwd") + 1], target)
  assert.equal(options.cwd, target)
  assert.equal(manifest.rows["MP-02"].checks.exact_path_entry.result.cwd_matches_requested, true)

  const mismatch = makeHarness("ordinary")
  mismatch.filesystem.directories.add(target)
  mismatch.options.expectedCwd = target
  await assert.rejects(() => mismatch.collector.collect(mismatch.options), (error) => {
    assert.equal(error.code, "cwd_mismatch")
    return true
  })
})

test("collector rejects arbitrary shutdown outcomes and premature deadlines", async () => {
  const wrongOutcome = makeHarness("path1", {
    results: {
      "MP-09/shutdown_idle_30m": {
        ...genericResult("MP-09", "shutdown_idle_30m", "path1"),
        observed_outcome: "non-empty-but-wrong",
      },
    },
  })
  await assert.rejects(() => wrongOutcome.collector.collect(wrongOutcome.options), (error) => {
    assert.equal(error.code, "shutdown_outcome_mismatch")
    return true
  })

  const premature = makeHarness("path1", {
    results: {
      "MP-09/shutdown_custom": {
        ...genericResult("MP-09", "shutdown_custom", "path1"),
        observed_delay_seconds: 599,
      },
    },
  })
  await assert.rejects(() => premature.collector.collect(premature.options), (error) => {
    assert.equal(error.code, "shutdown_delay_invalid")
    return true
  })
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

test("collector rejects control-file evidence that leaves its parent unavailable as a workspace", async () => {
  const harness = makeHarness("path1", {
    results: {
      "MP-03/control_file_protection": {
        observed: true,
        control_file_denied: true,
        parent_workspace_accessible: false,
        sibling_accessible: true,
      },
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "probe_assertion_failed")
    assert.equal(error.rowId, "MP-03")
    assert.equal(error.key, "parent_workspace_accessible")
    return true
  })
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

test("signed kernel release identity must bind the selected binary to the reviewed commit", async () => {
  const harness = makeHarness("ordinary")
  harness.filesystem.files.set(`${KERNEL_RELEASE_ROOT}/usr/lib/chariox/release-manifest.json`, JSON.stringify({
    schemaVersion: 2,
    sourceCommit: "a".repeat(40),
    sourceTree: SOURCE_TREE,
    artifacts: [{ name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: KERNEL_DIGEST }],
  }))
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "kernel_release_identity_mismatch")
    return true
  })
})

test("Bubblewrap ancestry is a product-boundary failure", async () => {
  const harness = makeHarness("path1", {
    results: {
      "MP-01/provider_ancestry": {
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
    assert.equal(error.rowId, "MP-01")
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

test("MP-10 caller boundary booleans without kernel provider-turn provenance fail closed", async () => {
  const harness = makeHarness("ordinary", {
    results: {
      "MP-10/capture_boundary": {
        observed: true,
        boundary: "official-provider-turn",
        inside_provider_turn: true,
        independent: true,
      },
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "capture_provenance_missing")
    return true
  })
})

test("collector rejects a probe proof for a different prompt than its capture binding", async () => {
  const harness = makeHarness("ordinary", {
    providerTurnBindingFactory: async () => ({
      socketPath: LIVE_KERNEL_SOCKET,
      kernelIdentity: { ...LIVE_KERNEL_IDENTITY, transport: "local-unix-ipc" },
      initialProof: CAPTURE_PROVENANCE,
      async finish() { return { ...CAPTURE_PROVENANCE, prompt_id: "prompt-foreign" } },
    }),
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "capture_provenance_mismatch")
    return true
  })
})

test("remote-command capture boundary is rejected without its own authority", async () => {
  const harness = makeHarness("ordinary")
  harness.options.boundary = "remote-command"
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "capture_boundary_invalid")
    return true
  })
})

test("caller-supplied probe executables are rejected before collection", async () => {
  const harness = makeHarness("ordinary")
  harness.options.probeCommand = "/tmp/fake-probe"
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "probe_command_unsupported")
    return true
  })
})

test("malformed probe output fails closed", async () => {
  const harness = makeHarness("ordinary", {
    command(command, args) {
      if (command === process.execPath && args[0].endsWith("managed-ordinary-parity-probe.mjs") && args.includes("home_access")) {
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
      if (command === KERNEL_BINARY && args[0] === "--version") {
        throw Object.assign(new Error("probe timeout"), { code: "ETIMEDOUT", timedOut: true })
      }
    },
  })
  await assert.rejects(() => harness.collector.collect(harness.options), (error) => {
    assert.equal(error.code, "command_timeout")
    return true
  })
})

test("default runner preserves real execFile rejection metadata", async () => {
  await assert.rejects(
    () => defaultRunCommand(process.execPath, ["-e", "process.exit(23)"], { timeout: 1_000 }),
    (error) => {
      assert.equal(error.code, 23)
      assert.equal(error.signal, null)
      assert.equal(error.killed, false)
      assert.equal(error.timedOut, undefined)
      return true
    },
  )
  await assert.rejects(
    () => defaultRunCommand(process.execPath, ["-e", "setTimeout(() => {}, 1_000)"], { timeout: 20 }),
    (error) => {
      assert.equal(error.code, null)
      assert.equal(error.signal, "SIGTERM")
      assert.equal(error.killed, true)
      assert.equal(error.timedOut, true)
      return true
    },
  )
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
      "MP-08/project_setup": {
        observed: true,
        project_setup_ok: true,
        project_setup_proof: projectSetupProofFixture(),
        token: "supersecret-token",
      },
    },
  })
  const second = makeHarness("ordinary", {
    results: {
      "MP-08/project_setup": {
        observed: true,
        project_setup_ok: true,
        project_setup_proof: projectSetupProofFixture(),
        token: "supersecret-token",
      },
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

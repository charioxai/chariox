import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import dns from "node:dns/promises"
import { chmod, mkdtemp, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import { dirname, join } from "node:path"
import { test } from "node:test"

import {
  COLLECTOR_CHECKS,
  createParityCollector,
  defaultRunCommand,
} from "./managed-ordinary-parity-collector.mjs"
import { normalizeMountInfo } from "./managed-ordinary-parity-probe.mjs"
import {
  compareManifests,
  validateManifest,
} from "./managed-ordinary-parity-matrix.mjs"

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

function memoryFilesystem() {
  const files = new Map()
  const directories = new Set()
  return {
    files,
    directories,
    async mkdir(directory) { directories.add(directory) },
    async readFile(file) {
      if (!files.has(file)) throw Object.assign(new Error(`missing fixture file ${file}`), { code: "ENOENT" })
      return files.get(file)
    },
    async realpath(file) {
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
  filesystem.files.set(`${KERNEL_RELEASE_ROOT}/usr/lib/chariox/release-manifest.json`, JSON.stringify({
    schemaVersion: 2,
    sourceCommit: REVIEWED_COMMIT,
    sourceTree: SOURCE_TREE,
    artifacts: [{ name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: KERNEL_DIGEST }],
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
    kernelBinary: KERNEL_BINARY,
    kernelReleaseRoot: KERNEL_RELEASE_ROOT,
    kernelReleaseDigest: RELEASE_DIGEST,
    kernelReleasePublicKey: KERNEL_RELEASE_PUBLIC_KEY,
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

function stableFixture(value) {
  if (Array.isArray(value)) return value.map(stableFixture)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stableFixture(value[key])]))
}

function fixtureFingerprint(value) {
  const bytes = typeof value === "string" ? value : JSON.stringify(stableFixture(value))
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

async function childFacts() {
  const script = [
    "const fs = require('node:fs')",
    "const status = fs.readFileSync('/proc/self/status', 'utf8')",
    "const limits = fs.readFileSync('/proc/self/limits', 'utf8').split('\\n').filter(Boolean).map((line) => line.trim().replace(/\\s+/g, ' ')).sort()",
    "const capEff = /^CapEff:\\s*([0-9a-f]+)$/im.exec(status)?.[1]?.toLowerCase()",
    "process.stdout.write(JSON.stringify({ uid: process.getuid(), gid: process.getgid(), umask: process.umask().toString(8).padStart(4, '0'), cap_eff: capEff, limits }))",
  ].join(";")
  const result = await defaultRunCommand(process.execPath, ["-e", script])
  assert.equal(result.code, 0, result.stderr)
  return JSON.parse(result.stdout)
}

async function listenForProbe() {
  const server = net.createServer((socket) => socket.end())
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  return { server, port: server.address().port }
}

test("collector invokes the repo-owned probe for every required non-source check", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-integration-"))
  const runtimeRoot = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-runtime-"))
  const controlRoot = await mkdtemp(join(os.tmpdir(), "chariox-parity-control-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  context.after(() => rm(runtimeRoot, { recursive: true, force: true }))
  context.after(() => rm(controlRoot, { recursive: true, force: true }))
  await chmod(root, 0o755)
  await chmod(runtimeRoot, 0o755)
  await chmod(controlRoot, 0o755)
  const probePath = join(root, "apps/cli/scripts/managed-ordinary-parity-probe.mjs")
  const releaseVerifierPath = join(root, "deploy/managed-kernel/verify-image-release.mjs")
  const relayProtocolPath = join(root, "apps/kernel/src/transport/relay_peer.rs")
  await mkdir(dirname(probePath), { recursive: true })
  await mkdir(dirname(releaseVerifierPath), { recursive: true })
  await mkdir(dirname(relayProtocolPath), { recursive: true })
  await writeFile(probePath, await readFile(new URL("./managed-ordinary-parity-probe.mjs", import.meta.url)))
  await writeFile(releaseVerifierPath, "#!/usr/bin/env node\nprocess.exit(0)\n", { mode: 0o755 })
  await chmod(releaseVerifierPath, 0o755)
  await writeFile(relayProtocolPath, "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 54;\n")

  const gitEnv = { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", LC_ALL: "C" }
  for (const args of [
    ["init", "--quiet"],
    ["config", "user.name", "parity-probe-integration"],
    ["config", "user.email", "parity-probe-integration@example.invalid"],
    ["add", "."],
    ["commit", "--quiet", "-m", "probe integration fixture"],
  ]) {
    const result = await defaultRunCommand("git", args, { cwd: root, env: gitEnv })
    assert.equal(result.code, 0, result.stderr)
  }
  const reviewedCommit = (await defaultRunCommand("git", ["rev-parse", "HEAD"], { cwd: root, env: gitEnv })).stdout.trim()
  const sourceTree = (await defaultRunCommand("git", ["rev-parse", "HEAD^{tree}"], { cwd: root, env: gitEnv })).stdout.trim()

  const home = join(runtimeRoot, "home")
  const charioxHome = join(home, ".chariox")
  const repositoryParent = join(runtimeRoot, "repository-parent")
  await mkdir(charioxHome, { recursive: true })
  await mkdir(repositoryParent, { recursive: true })
  await chmod(home, 0o755)
  await chmod(charioxHome, 0o755)
  await chmod(repositoryParent, 0o777)

  const releaseRoot = join(runtimeRoot, "release-root")
  const kernelBinary = join(releaseRoot, "usr/local/bin/chariox-kernel")
  const releaseManifestPath = join(releaseRoot, "usr/lib/chariox/release-manifest.json")
  const releasePublicKey = join(runtimeRoot, "release-public-key")
  await mkdir(dirname(kernelBinary), { recursive: true })
  await mkdir(dirname(releaseManifestPath), { recursive: true })
  const kernelBytes = "#!/usr/bin/env node\nprocess.stdout.write(process.argv[2] === '--version' ? 'chariox-kernel build-20260920\\n' : '332\\n')\n"
  await writeFile(kernelBinary, kernelBytes, { mode: 0o755 })
  await chmod(kernelBinary, 0o755)
  const kernelDigest = `sha256:${createHash("sha256").update(kernelBytes).digest("hex")}`
  const releaseDigest = `sha256:${"a".repeat(64)}`
  await writeFile(releaseManifestPath, JSON.stringify({
    schemaVersion: 2,
    sourceCommit: reviewedCommit,
    sourceTree,
    artifacts: [{ name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: kernelDigest }],
  }))
  await writeFile(releasePublicKey, "integration-public-key\n")
  await chmod(releaseRoot, 0o755)

  const facts = await childFacts()
  const mountFingerprint = fixtureFingerprint(normalizeMountInfo(await readFile("/proc/self/mountinfo", "utf8")))
  const environmentBaseline = {
    home_fingerprint: fixtureFingerprint(home),
    chariox_home_fingerprint: fixtureFingerprint(charioxHome),
    cwd_fingerprint: fixtureFingerprint(root),
    uid: facts.uid,
    gid: facts.gid,
  }
  const network = await listenForProbe()
  context.after(() => network.server.close())
  const addresses = await dns.lookup("127.0.0.1", { all: true, verbatim: true })
  const addressFamilies = [...new Set(addresses.map((entry) => entry.family).sort())]
  const networkObserved = { endpoint: "integration-network", port: network.port, address_families: addressFamilies, reachable: true }
  const attachment = join(runtimeRoot, "attachment.bin")
  await writeFile(attachment, "attachment\n")
  await chmod(attachment, 0o666)
  const attachmentMode = ((await stat(attachment)).mode & 0o7777).toString(8).padStart(4, "0")
  const controlFile = join(controlRoot, "control-secret")
  const controlSibling = join(controlRoot, "control-sibling")
  await writeFile(controlFile, "control\n", { mode: 0o600 })
  await writeFile(controlSibling, "sibling\n", { mode: 0o644 })
  await chmod(controlFile, 0o600)
  await chmod(controlSibling, 0o644)

  const shutdownEvidence = Object.fromEntries([...COLLECTOR_CHECKS["MP-10"]].map((checkId) => {
    const trigger = checkId.slice("shutdown_".length)
    return [trigger, {
      observed: true,
      trigger,
      observed_outcome: "ordinary-no-managed-shutdown",
      managed_policy: false,
      shutdown_evidence: true,
      measured_from_last_agent_finished: trigger === "idle_15m" || trigger === "idle_30m",
      receipt_id: `integration-${trigger}`,
    }]
  }))
  const lifecycleEvidence = Object.fromEntries(Object.keys({
    reconnect_orphan_recovery: ["reconnect_ok", "orphan_recovered"],
    restart_recovery: ["restart_recovered"],
    reconnect_history_result_identity: ["history_preserved", "result_identity_preserved"],
    queued_prompts: ["queued_prompt_preserved", "queued_prompt_advanced"],
    active_turn_state: ["active_turn_state_preserved"],
  }).map((checkId) => [checkId, {
    observed: true,
    evidence_id: `integration-${checkId}`,
    ...Object.fromEntries(({
      reconnect_orphan_recovery: ["reconnect_ok", "orphan_recovered"],
      restart_recovery: ["restart_recovered"],
      reconnect_history_result_identity: ["history_preserved", "result_identity_preserved"],
      queued_prompts: ["queued_prompt_preserved", "queued_prompt_advanced"],
      active_turn_state: ["active_turn_state_preserved"],
    }[checkId]).map((key) => [key, true])),
  }]))
  const providerEnvironment = {
    ...gitEnv,
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "safe.directory",
    GIT_CONFIG_VALUE_0: root,
    HOME: home,
    CHARIOX_HOME: charioxHome,
    CHARIOX_PARITY_PROVIDER: "codex",
    CHARIOX_PARITY_PROVIDER_PROCESS_OBSERVED: "1",
    CHARIOX_PARITY_PROVIDER_IDENTITY_JSON: JSON.stringify({ observed: true, process_observed: true, name: "codex", version: "integration-provider-1", executable: "codex", official: true }),
    CHARIOX_PARITY_WORKER_EVIDENCE_JSON: JSON.stringify({ observed: true, fresh_worker: true, worker_id: "integration-worker", start_time: "integration" }),
    CHARIOX_PARITY_ANCESTRY_EVIDENCE_JSON: JSON.stringify({ observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true, ancestry_complete: true, evidence_id: "integration-ancestry" }),
    CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({ observed: true, boundary: "official-provider-turn", inside_provider_turn: true, independent: true }),
    CHARIOX_PARITY_ORDINARY_ENVIRONMENT_JSON: JSON.stringify(environmentBaseline),
    CHARIOX_PARITY_ORDINARY_MOUNT_FINGERPRINT: mountFingerprint,
    CHARIOX_PARITY_ORDINARY_PRIVILEGE_JSON: JSON.stringify({ cap_eff: facts.cap_eff, umask: facts.umask, uid: facts.uid, gid: facts.gid }),
    CHARIOX_PARITY_PRIVILEGE_EVIDENCE_JSON: JSON.stringify({ observed: true, no_new_privs: false, evidence_id: "integration-privilege" }),
    CHARIOX_PARITY_NETWORK_PROBE_JSON: JSON.stringify({ name: "integration-network", host: "127.0.0.1", port: network.port }),
    CHARIOX_PARITY_ORDINARY_NETWORK_JSON: JSON.stringify({ network_fingerprint: fixtureFingerprint(networkObserved), reachable: true, address_families: addressFamilies }),
    CHARIOX_PARITY_PACKAGE_PROBE_JSON: JSON.stringify({ command: process.execPath, args: ["--version"], install_command: process.execPath, install_args: ["--version"], tool: "node", operation: "version-probe", expected_exit_code: 0, install_expected_exit_code: 0 }),
    CHARIOX_PARITY_SESSION_AGENT_EVIDENCE_JSON: JSON.stringify({ observed: true, session_created: true, agent_created: true, official_command: true, session_id: "integration-session", agent_id: "integration-agent" }),
    CHARIOX_PARITY_PROJECT_SETUP_EVIDENCE_JSON: JSON.stringify({ observed: true, project_setup_ok: true, project_identity: "integration-project" }),
    CHARIOX_PARITY_LIFECYCLE_EVIDENCE_JSON: JSON.stringify(lifecycleEvidence),
    CHARIOX_PARITY_ATTACHMENT_PATH: attachment,
    CHARIOX_PARITY_ORDINARY_ATTACHMENT_JSON: JSON.stringify({ mode: attachmentMode, cap_eff: facts.cap_eff }),
    CHARIOX_PARITY_CONTROL_FILE: controlFile,
    CHARIOX_PARITY_CONTROL_SIBLING: controlSibling,
    CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON: JSON.stringify({ observed: true, control_file_denied: true, sibling_accessible: true }),
    CHARIOX_PARITY_ORDINARY_FILESYSTEM_PERMISSIONS_JSON: JSON.stringify({ mode: "0755", uid: 0, gid: 0 }),
    CHARIOX_PARITY_ORDINARY_RESOURCE_LIMITS_FINGERPRINT: fixtureFingerprint(facts.limits),
    CHARIOX_PARITY_STRUCTURED_ERROR_EVIDENCE_JSON: JSON.stringify({ observed: true, structured_errors: true, error_code: "integration_error", retryable: false }),
    CHARIOX_PARITY_PROTOCOL_EVIDENCE_JSON: JSON.stringify({ observed: true, protocol_behavior_ok: true, protocol_identity: "integration-protocol" }),
    CHARIOX_PARITY_CLEANUP_RESULT_JSON: JSON.stringify({ observed: true, owned_processes_gone: true, owned_artifacts_removed: true, foreign_processes_untouched: true, cleanup_complete: true, receipt_id: "integration-cleanup" }),
    CHARIOX_PARITY_RELEASE_EVIDENCE_JSON: JSON.stringify({ observed: true, ordinary_release_not_applicable: true, signed_release_present: false, signature_verified: false, atomic_activation: false, rollback_verified: false, release_digest: "not-applicable", receipt_id: "integration-release" }),
    CHARIOX_PARITY_SHUTDOWN_EVIDENCE_JSON: JSON.stringify(shutdownEvidence),
  }

  const calls = []
  const runCommand = async (command, args, options = {}) => {
    calls.push([command, [...args]])
    if (command === process.execPath && args[0] === probePath) {
      return await defaultRunCommand(command, args, { ...options, env: providerEnvironment })
    }
    if (command === process.execPath && args[0] === releaseVerifierPath) return { code: 0, signal: null, stdout: "", stderr: "" }
    if (command === kernelBinary && args[0] === "--version") return { code: 0, signal: null, stdout: "chariox-kernel build-20260920\n", stderr: "" }
    if (command === kernelBinary && args[0] === "--print-local-daemon-protocol-version") return { code: 0, signal: null, stdout: "332\n", stderr: "" }
    if (command === "codex" && args[0] === "--version") return { code: 0, signal: null, stdout: "codex integration-provider-1\n", stderr: "" }
    return defaultRunCommand(command, args, options)
  }
  const outputPath = join(runtimeRoot, "evidence", "ordinary.json")
  const evidenceDir = join(runtimeRoot, "evidence", "commands")
  const collector = createParityCollector({ runCommand, processApi: { platform: "linux", pid: process.pid, cwd: () => root } })
  const manifest = await collector.collect({
    topology: "ordinary",
    reviewedCommit,
    buildId: "chariox-kernel build-20260920",
    kernelProtocol: 332,
    relayProtocol: 54,
    provider: "codex",
    providerCommand: "codex",
    kernelBinary,
    kernelReleaseRoot: releaseRoot,
    kernelReleaseDigest: releaseDigest,
    kernelReleasePublicKey: releasePublicKey,
    boundary: "official-provider-turn",
    outputPath,
    evidenceDir,
    sourceRoot: root,
    signingKey: SIGNING_KEY,
    timeoutMs: 5_000,
  })
  assert.equal(validateManifest(manifest, {
    expectedTopology: "ordinary",
    expectedReviewedCommit: reviewedCommit,
    expectedBuildId: "chariox-kernel build-20260920",
    signingKey: SIGNING_KEY,
  }).ok, true)
  const probeCalls = calls.filter(([command, args]) => command === process.execPath && args[0] === probePath)
  const expectedProbeCalls = Object.values(COLLECTOR_CHECKS).flat().length - 1
  assert.equal(probeCalls.length, expectedProbeCalls)
  assert.equal(new Set(probeCalls.map(([, args]) => `${args[args.indexOf("--parity-row") + 1]}/${args[args.indexOf("--parity-check") + 1]}`)).size, expectedProbeCalls)
  assert.ok((await readFile(`${evidenceDir}/ordinary/MP-04/mount_visibility/probe.json`, "utf8")).includes("stdout_sha256"))
})

#!/usr/bin/env node

import { execFile } from "node:child_process"
import { createHash, createHmac, timingSafeEqual } from "node:crypto"
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)

export const SNAPSHOT_SCHEMA = "chariox.managed-ordinary-path1-runtime-parity/v1"
export const ALLOWED_TOPOLOGIES = ["ordinary", "path1"]
export const ALLOWED_CAPTURE_BOUNDARIES = ["official-provider-turn", "remote-command"]
export const OFFICIAL_PROVIDERS = ["codex", "claude", "opencode"]

export const MANDATORY_SHUTDOWN_TRIGGERS = [
  "agents_done",
  "idle_15m",
  "idle_30m",
  "minimum_3h",
  "manual",
  "custom",
  "explicit_lifecycle_reconciliation",
  "deployment_reconciliation",
]

export const REQUIRED_RESULT_FIELDS = [
  "exact_cwd",
  "arbitrary_accessible_directory",
  "home_access",
  "tmp_access",
  "post_enrollment_repository",
  "git_file_terminal",
  "provider_ancestry",
  "managed_isolation_environment",
  "mount_visibility",
  "privilege_state",
  "network_reachability",
  "package_tool_installation",
  "official_provider_identity",
  "reconnect_history_result_identity",
  "errors",
  "cleanup",
  ...MANDATORY_SHUTDOWN_TRIGGERS.map((trigger) => "shutdown_" + trigger),
]

const FORBIDDEN_ENVIRONMENT_MARKERS = [
  "CHARIOX_MANAGED_PROVIDER_ISOLATION",
  "CHARIOX_MANAGED_PROVIDER_BWRAP",
  "CHARIOX_MANAGED_PROVIDER_BWRAP_ARGS",
]
const SAFE_IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9_.:/-]{0,127}$/
const REVIEWED_COMMIT = /^[0-9a-f]{40}$/i
const MAX_COMMAND_OUTPUT = 16 * 1024
const MAX_PROBE_TIMEOUT_MS = 15_000

function stableValue(value) {
  if (Array.isArray(value)) return value.map(stableValue)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(
    Object.keys(value)
      .filter((key) => value[key] !== undefined)
      .sort()
      .map((key) => [key, stableValue(value[key])]),
  )
}

export function canonicalJson(value) {
  return JSON.stringify(stableValue(value))
}

function payloadWithoutSignature(snapshot) {
  const { signature: _signature, ...payload } = snapshot
  return payload
}

function keyId(signingKey) {
  return "sha256:" + createHash("sha256").update(signingKey).digest("hex")
}

function signPayload(snapshot, signingKey) {
  return createHmac("sha256", signingKey)
    .update(canonicalJson(payloadWithoutSignature(snapshot)))
    .digest("base64url")
}

function validSigningKey(signingKey) {
  return Buffer.isBuffer(signingKey) && signingKey.length >= 16
}

function validTopology(topology) {
  return ALLOWED_TOPOLOGIES.includes(topology)
}

function validCommit(commit) {
  return typeof commit === "string" && REVIEWED_COMMIT.test(commit)
}

function validBuildId(buildId) {
  return typeof buildId === "string" && buildId.length >= 8 && buildId.length <= 256
}

function validBoundary(boundary) {
  return ALLOWED_CAPTURE_BOUNDARIES.includes(boundary)
}

function result(status, comparison = null, evidence = {}, reason = null) {
  return {
    status,
    comparison,
    evidence,
    ...(reason ? { reason } : {}),
  }
}

function passed(comparison, evidence = {}) {
  return result("passed", comparison, evidence)
}

function failed(reason, comparison = null, evidence = {}) {
  return result("failed", comparison, evidence, reason)
}

function missing(reason) {
  return result("missing", null, {}, reason)
}

export function createSignedSnapshot({
  reviewedCommit,
  buildId,
  topology,
  workerFresh,
  provider,
  capture,
  results,
  signingKey,
  capturedAt = new Date().toISOString(),
}) {
  if (!validCommit(reviewedCommit)) throw new Error("invalid reviewed commit")
  if (!validBuildId(buildId)) throw new Error("invalid reviewed build id")
  if (!validTopology(topology)) throw new Error("invalid declared topology")
  if (!validSigningKey(signingKey)) throw new Error("invalid signing key")
  const snapshot = {
    schema: SNAPSHOT_SCHEMA,
    snapshot_kind: "runtime_parity",
    captured_at: capturedAt,
    reviewed_commit: reviewedCommit,
    reviewed_build_id: buildId,
    declared_topology: topology,
    worker_fresh: workerFresh === true,
    provider: provider ?? {},
    capture: capture ?? {},
    results: results ?? {},
  }
  snapshot.signature = {
    algorithm: "hmac-sha256",
    key_id: keyId(signingKey),
    value: signPayload(snapshot, signingKey),
  }
  return snapshot
}

export function verifySnapshotSignature(snapshot, signingKey) {
  if (!validSigningKey(signingKey)) return { ok: false, reason: "signing_key_missing" }
  const signature = snapshot?.signature
  if (
    !signature
    || signature.algorithm !== "hmac-sha256"
    || signature.key_id !== keyId(signingKey)
    || typeof signature.value !== "string"
  ) {
    return { ok: false, reason: "signature_metadata_invalid" }
  }
  const expected = signPayload(snapshot, signingKey)
  const expectedBytes = Buffer.from(expected)
  const actualBytes = Buffer.from(signature.value)
  if (expectedBytes.length !== actualBytes.length
    || !timingSafeEqual(expectedBytes, actualBytes)) {
    return { ok: false, reason: "signature_invalid" }
  }
  return { ok: true }
}

function hasRuntimeCapture(snapshot) {
  return snapshot?.capture?.runtimeProbe === true
    && snapshot.capture.invokedInsideProviderTurn === true
    && validBoundary(snapshot.capture.boundary)
}

function providerIsValid(provider) {
  return provider
    && OFFICIAL_PROVIDERS.includes(provider.name)
    && typeof provider.version === "string"
    && provider.version.length > 0
    && typeof provider.executable === "string"
    && provider.executable.length > 0
    && provider.official === true
}

function resultIsPassed(value) {
  return value && value.status === "passed" && value.comparison !== null
}

function unique(values) {
  return [...new Set(values)]
}

export function validateSnapshot(snapshot, {
  expectedTopology,
  expectedReviewedCommit,
  expectedBuildId,
  signingKey,
  allowFixture = false,
  requirePassed = false,
} = {}) {
  const errors = []
  if (!snapshot || typeof snapshot !== "object") {
    return { ok: false, errors: ["snapshot_not_object"] }
  }
  if (snapshot.schema !== SNAPSHOT_SCHEMA) errors.push("schema_mismatch")
  if (snapshot.snapshot_kind !== "runtime_parity") errors.push("snapshot_kind_mismatch")
  if (!validTopology(snapshot.declared_topology)) errors.push("topology_invalid")
  if (expectedTopology && snapshot.declared_topology !== expectedTopology) {
    errors.push("topology_mismatch")
  }
  if (!validCommit(snapshot.reviewed_commit)) errors.push("reviewed_commit_invalid")
  if (expectedReviewedCommit && snapshot.reviewed_commit !== expectedReviewedCommit) {
    errors.push("reviewed_commit_mismatch")
  }
  if (!validBuildId(snapshot.reviewed_build_id)) errors.push("reviewed_build_id_invalid")
  if (expectedBuildId && snapshot.reviewed_build_id !== expectedBuildId) {
    errors.push("reviewed_build_id_mismatch")
  }
  if (snapshot.declared_topology === "path1" && snapshot.worker_fresh !== true) {
    errors.push("fresh_worker_required")
  }
  if (!hasRuntimeCapture(snapshot)) errors.push("capture_not_runtime")
  if (snapshot.capture?.fixture === true && !allowFixture) errors.push("fixture_not_allowed")
  if (!providerIsValid(snapshot.provider)) errors.push("official_provider_identity_invalid")
  const signature = verifySnapshotSignature(snapshot, signingKey)
  if (!signature.ok) errors.push(signature.reason)

  if (!snapshot.results || typeof snapshot.results !== "object") {
    errors.push("results_missing")
  } else {
    for (const field of REQUIRED_RESULT_FIELDS) {
      const value = snapshot.results[field]
      if (!value) {
        errors.push("missing_result:" + field)
      } else if (requirePassed && !resultIsPassed(value)) {
        errors.push("result_not_passed:" + field)
      }
    }
  }

  if (snapshot.declared_topology === "path1" && snapshot.results) {
    const ancestry = snapshot.results.provider_ancestry?.comparison
    const isolation = snapshot.results.managed_isolation_environment?.comparison
    if (ancestry?.bwrap_ancestor !== false) errors.push("path1_bwrap_ancestor")
    if (isolation?.managed_marker_absent !== true) errors.push("path1_managed_marker")
    if (isolation?.bwrap_environment_absent !== true) errors.push("path1_bwrap_environment")
  }

  return { ok: errors.length === 0, errors: unique(errors) }
}

function compareValue(left, right) {
  return canonicalJson(left) === canonicalJson(right)
}

export function compareSnapshots(ordinary, path1, {
  expectedReviewedCommit,
  expectedBuildId,
  signingKey,
  allowFixture = false,
} = {}) {
  const errors = []
  if (!expectedReviewedCommit) errors.push("expected_reviewed_commit_missing")
  else if (!validCommit(expectedReviewedCommit)) errors.push("expected_reviewed_commit_invalid")
  if (!expectedBuildId) errors.push("expected_reviewed_build_id_missing")
  const ordinaryValidation = validateSnapshot(ordinary, {
    expectedTopology: "ordinary",
    expectedReviewedCommit,
    expectedBuildId,
    signingKey,
    allowFixture,
    requirePassed: true,
  })
  const path1Validation = validateSnapshot(path1, {
    expectedTopology: "path1",
    expectedReviewedCommit,
    expectedBuildId,
    signingKey,
    allowFixture,
    requirePassed: true,
  })
  errors.push(...ordinaryValidation.errors.map((error) => "ordinary:" + error))
  errors.push(...path1Validation.errors.map((error) => "path1:" + error))
  if (errors.length > 0) return { ok: false, errors: unique(errors), comparedFields: 0 }

  if (ordinary.signature.key_id !== path1.signature.key_id) {
    errors.push("signature_key_mismatch")
  }
  if (ordinary.provider.name !== path1.provider.name) errors.push("provider_name_mismatch")
  if (ordinary.provider.version !== path1.provider.version) errors.push("provider_version_mismatch")
  if (ordinary.provider.executable !== path1.provider.executable) errors.push("provider_executable_mismatch")
  let comparedFields = 0
  for (const field of REQUIRED_RESULT_FIELDS) {
    comparedFields += 1
    if (!compareValue(ordinary.results[field].comparison, path1.results[field].comparison)) {
      errors.push("different_result:" + field)
    }
  }
  return { ok: errors.length === 0, errors: unique(errors), comparedFields }
}

function fixtureComparison(field) {
  if (field === "exact_cwd") return { logical_workspace: "selected-workspace", exact_match: true }
  if (field === "arbitrary_accessible_directory") return { logical_root: "arbitrary-accessible", create: true, read: true, write: true }
  if (field === "home_access") return { logical_root: "home", create: true, read: true, write: true }
  if (field === "tmp_access") return { logical_root: "tmp", create: true, read: true, write: true }
  if (field === "post_enrollment_repository") return { created_after_enrollment: true, repository_readable: true, removed: true }
  if (field === "git_file_terminal") return { git: true, file: true, terminal: true }
  if (field === "provider_ancestry") return { provider_observed: true, bwrap_ancestor: false, fresh_worker: true }
  if (field === "managed_isolation_environment") return { managed_marker_absent: true, bwrap_environment_absent: true }
  if (field === "mount_visibility") return { home: true, tmp: true, workspace: true, repository: true }
  if (field === "privilege_state") return { no_new_privs: false, cap_eff: "0", umask: "0022" }
  if (field === "network_reachability") return { reachable: true, endpoint: "parity-network" }
  if (field === "package_tool_installation") return { operation: "permitted-installation-probe", succeeded: true, tool: "node" }
  if (field === "official_provider_identity") return { official: true, provider: "codex", version: "fixture-provider-1" }
  if (field === "reconnect_history_result_identity") return { reconnected: true, history_persisted: true, result_identity: "fixture-result" }
  if (field === "errors") return { structured: true, code: "fixture_error", retryable: false }
  if (field === "cleanup") return { temporary_data_removed: true, control_state_separate: true, no_failure: true }
  if (field.startsWith("shutdown_")) {
    const trigger = field.slice("shutdown_".length)
    return {
      trigger,
      observed: true,
      outcome: trigger === "manual" ? "retain" : "stop",
    }
  }
  return { passed: true }
}

export function makeFixtureResults(overrides = {}) {
  const results = {}
  for (const field of REQUIRED_RESULT_FIELDS) {
    results[field] = passed(fixtureComparison(field), { fixture: true })
  }
  for (const [field, value] of Object.entries(overrides)) {
    if (value === null) delete results[field]
    else if (value.status || value.comparison || value.reason) results[field] = value
    else results[field] = passed(value, { fixture: true })
  }
  return results
}

function safeReason(error) {
  const code = error?.code
  if (typeof code === "string" && SAFE_IDENTIFIER.test(code)) return code
  if (error?.name === "AbortError") return "aborted"
  return "probe_error"
}

function sha256Text(value) {
  return "sha256:" + createHash("sha256").update(String(value)).digest("hex")
}

function parseJsonEnv(name) {
  const raw = process.env[name]
  if (!raw) return null
  try {
    return JSON.parse(raw)
  } catch {
    return { __invalid: true }
  }
}

function parseBoolean(value) {
  return value === true || value === "1" || value === "true"
}

function safeIdentifier(value, fallback = null) {
  return typeof value === "string" && SAFE_IDENTIFIER.test(value) ? value : fallback
}

async function runCommand(command, args, options = {}) {
  try {
    const result = await execFileAsync(command, args, {
      cwd: options.cwd,
      env: options.env,
      timeout: Math.min(options.timeoutMs ?? MAX_PROBE_TIMEOUT_MS, MAX_PROBE_TIMEOUT_MS),
      maxBuffer: MAX_COMMAND_OUTPUT,
      windowsHide: true,
    })
    return {
      code: 0,
      stdoutHash: sha256Text(result.stdout ?? ""),
      stderrHash: sha256Text(result.stderr ?? ""),
    }
  } catch (error) {
    return {
      code: Number.isInteger(error?.code) ? error.code : null,
      signal: typeof error?.signal === "string" ? error.signal : null,
      stdoutHash: sha256Text(error?.stdout ?? ""),
      stderrHash: sha256Text(error?.stderr ?? ""),
      reason: safeReason(error),
    }
  }
}

async function probeWritableDirectory(target, label, scratchRoot) {
  if (!target) return missing(label + "_path_missing")
  let targetPath
  try {
    targetPath = await realpath(target)
    const metadata = await stat(targetPath)
    if (!metadata.isDirectory()) return failed(label + "_not_directory")
  } catch (error) {
    return failed(safeReason(error))
  }
  let scratch = null
  try {
    scratch = await mkdtemp(path.join(targetPath, ".chariox-path1-parity-"))
    const file = path.join(scratch, "probe")
    const marker = "chariox-path1-parity-" + label
    await writeFile(file, marker, "utf8")
    const observed = await readFile(file, "utf8")
    const same = observed === marker
    await rm(scratch, { recursive: true, force: false })
    scratch = null
    return same
      ? passed({ logical_root: label, create: true, read: true, write: true }, {
          path_hash: sha256Text(targetPath),
          marker_hash: sha256Text(marker),
        })
      : failed(label + "_readback_mismatch")
  } catch (error) {
    if (scratch) await rm(scratch, { recursive: true, force: true }).catch(() => {})
    return failed(safeReason(error))
  }
}

async function probeRepository(parent, scratchRoot) {
  if (!parent) return missing("repository_parent_missing")
  let repository = null
  try {
    const parentPath = await realpath(parent)
    repository = await mkdtemp(path.join(parentPath, ".chariox-path1-parity-repository-"))
    const initialized = await runCommand("git", ["init", "--quiet", repository])
    const file = path.join(repository, "created-after-enrollment.txt")
    await writeFile(file, "created-after-enrollment\n", "utf8")
    const observed = await readFile(file, "utf8")
    const status = await runCommand("git", ["-C", repository, "status", "--porcelain"])
    const terminal = await runCommand("sh", ["-c", "pwd >/dev/null && test -f created-after-enrollment.txt"], { cwd: repository })
    const ok = initialized.code === 0 && observed === "created-after-enrollment\n"
      && status.code === 0 && terminal.code === 0
    await rm(repository, { recursive: true, force: false })
    repository = null
    return ok
      ? passed({
          created_after_enrollment: true,
          repository_readable: true,
          removed: true,
        }, { parent_hash: sha256Text(parentPath) })
      : failed("repository_probe_failed")
  } catch (error) {
    if (repository) await rm(repository, { recursive: true, force: true }).catch(() => {})
    return failed(safeReason(error))
  }
}

async function processChain() {
  const chain = []
  let pid = process.pid
  const seen = new Set()
  while (Number.isInteger(pid) && pid > 0 && !seen.has(pid) && chain.length < 32) {
    seen.add(pid)
    let command = ""
    let parentPid = null
    try {
      command = (await readFile("/proc/" + pid + "/cmdline", "utf8")).replaceAll("\0", " ").trim()
      const statLine = await readFile("/proc/" + pid + "/stat", "utf8")
      const close = statLine.lastIndexOf(")")
      const after = close >= 0 ? statLine.slice(close + 2).trim().split(/\s+/) : []
      parentPid = Number(after[1])
    } catch {
      break
    }
    chain.push({ pid, command })
    if (!Number.isSafeInteger(parentPid) || parentPid <= 0 || parentPid === pid) break
    pid = parentPid
  }
  return chain
}

async function environmentMarkerSet(chain) {
  const found = new Set()
  for (const entry of chain) {
    try {
      const bytes = await readFile("/proc/" + entry.pid + "/environ")
      for (const item of bytes.toString("utf8").split("\0")) {
        const key = item.split("=", 1)[0]
        if (FORBIDDEN_ENVIRONMENT_MARKERS.includes(key)) found.add(key)
      }
    } catch {}
  }
  for (const key of FORBIDDEN_ENVIRONMENT_MARKERS) {
    if (Object.hasOwn(process.env, key)) found.add(key)
  }
  return found
}

async function probeProvider(topology, provider, workerFresh) {
  const chain = await processChain()
  const markerSet = await environmentMarkerSet(chain)
  const commands = chain.map((entry) => entry.command)
  const providerSeen = commands.some((command) => {
    const normalized = command.toLowerCase()
    return normalized.includes("/" + provider.toLowerCase())
      || normalized.startsWith(provider.toLowerCase() + " ")
      || normalized.includes(" " + provider.toLowerCase() + " ")
  })
  const bwrapAncestor = commands.some((command) => /(^|\s|\/)bwrap(?:\s|$)/i.test(command))
  const comparison = {
    provider_observed: providerSeen || parseBoolean(process.env.CHARIOX_PARITY_PROVIDER_PROCESS_OBSERVED),
    bwrap_ancestor: bwrapAncestor,
    fresh_worker: workerFresh === true,
  }
  const valid = comparison.provider_observed
    && !bwrapAncestor
    && (topology !== "path1" || comparison.fresh_worker)
  return valid
    ? passed(comparison, {
        process_count: chain.length,
        command_chain_hash: sha256Text(commands.join("\n")),
      })
    : failed(topology === "path1" && bwrapAncestor ? "path1_bwrap_ancestor" : "provider_ancestry_unverified", comparison, {
        process_count: chain.length,
        command_chain_hash: sha256Text(commands.join("\n")),
      })
}

async function probeIsolationEnvironment(markerSet) {
  const comparison = {
    managed_marker_absent: !markerSet.has("CHARIOX_MANAGED_PROVIDER_ISOLATION"),
    bwrap_environment_absent: !markerSet.has("CHARIOX_MANAGED_PROVIDER_BWRAP")
      && !markerSet.has("CHARIOX_MANAGED_PROVIDER_BWRAP_ARGS"),
  }
  return comparison.managed_marker_absent && comparison.bwrap_environment_absent
    ? passed(comparison)
    : failed("managed_isolation_environment_present", comparison)
}

async function probeMountVisibility(paths) {
  let mountInfo
  try {
    mountInfo = await readFile("/proc/self/mountinfo", "utf8")
  } catch (error) {
    return failed(safeReason(error))
  }
  const comparison = {}
  const evidence = { mount_namespace_hash: null }
  for (const [label, target] of Object.entries(paths)) {
    if (!target) {
      comparison[label] = false
      continue
    }
    try {
      await access(target)
      comparison[label] = true
    } catch {
      comparison[label] = false
    }
  }
  try {
    evidence.mount_namespace_hash = sha256Text(await readlink("/proc/self/ns/mnt"))
  } catch {}
  const mountInfoHash = sha256Text(mountInfo)
  evidence.mount_info_hash = mountInfoHash
  return Object.values(comparison).every(Boolean)
    ? passed(comparison, evidence)
    : failed("mount_visibility_missing", comparison, evidence)
}

async function probePrivilegeState() {
  try {
    const status = await readFile("/proc/self/status", "utf8")
    const noNewPrivs = status.match(/^NoNewPrivs:\s+(\d+)/m)?.[1]
    const capEff = status.match(/^CapEff:\s+([0-9a-f]+)/im)?.[1]
    const umask = process.umask().toString(8).padStart(4, "0")
    if (!noNewPrivs || !capEff) return missing("privilege_fields_missing")
    return passed({
      no_new_privs: noNewPrivs === "1",
      cap_eff: capEff,
      umask,
    })
  } catch (error) {
    return failed(safeReason(error))
  }
}

async function probeNetwork(networkProbe) {
  if (!networkProbe || typeof networkProbe !== "object") return missing("network_probe_not_configured")
  const port = Number(networkProbe.port)
  const host = typeof networkProbe.host === "string" ? networkProbe.host : ""
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535) return failed("network_probe_invalid")
  const reachable = await new Promise((resolve) => {
    const socket = net.createConnection({ host, port })
    const timer = setTimeout(() => {
      socket.destroy()
      resolve(false)
    }, MAX_PROBE_TIMEOUT_MS)
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.end()
      resolve(true)
    })
    socket.once("error", () => {
      clearTimeout(timer)
      socket.destroy()
      resolve(false)
    })
  })
  return reachable
    ? passed({ reachable: true, endpoint: safeIdentifier(networkProbe.name, "parity-network") })
    : failed("network_unreachable", { reachable: false, endpoint: safeIdentifier(networkProbe.name, "parity-network") })
}

async function probePackageTool(packageProbe) {
  if (!packageProbe || typeof packageProbe !== "object") return missing("package_probe_not_configured")
  if (typeof packageProbe.command !== "string" || !Array.isArray(packageProbe.args)) {
    return failed("package_probe_invalid")
  }
  const execution = await runCommand(packageProbe.command, packageProbe.args)
  const expected = Number.isInteger(packageProbe.expectedExitCode) ? packageProbe.expectedExitCode : 0
  const comparison = {
    operation: safeIdentifier(packageProbe.operation, "permitted-installation-probe"),
    succeeded: execution.code === expected,
    tool: safeIdentifier(packageProbe.tool, path.basename(packageProbe.command)),
  }
  return comparison.succeeded
    ? passed(comparison, { stdout_hash: execution.stdoutHash, stderr_hash: execution.stderrHash })
    : failed("package_probe_failed", comparison, { stdout_hash: execution.stdoutHash, stderr_hash: execution.stderrHash })
}

function probeProviderIdentity(provider, providerVersion, identity) {
  if (!identity || typeof identity !== "object" || identity.__invalid) {
    return missing("provider_identity_not_configured")
  }
  const name = identity.name ?? provider
  const version = identity.version ?? providerVersion
  const official = identity.official === true
  const observed = identity.observed === true
  const comparison = {
    official,
    provider: name,
    version: typeof version === "string" ? version : "",
  }
  return OFFICIAL_PROVIDERS.includes(name) && typeof version === "string" && version.length > 0 && official && observed
    ? passed(comparison, { executable_hash: sha256Text(identity.executable ?? name) })
    : failed("official_provider_identity_unverified", comparison)
}

function probeReconnectionHistory(value) {
  if (!value || typeof value !== "object" || value.__invalid) return missing("reconnect_history_not_configured")
  const comparison = {
    reconnected: value.reconnected === true,
    history_persisted: value.history_persisted === true,
    result_identity: typeof value.result_identity === "string" ? sha256Text(value.result_identity) : "",
  }
  return comparison.reconnected && comparison.history_persisted && comparison.result_identity
    ? passed(comparison)
    : failed("reconnect_history_result_invalid", comparison)
}

function probeErrors(value) {
  if (!value || typeof value !== "object" || value.__invalid) return missing("error_result_not_configured")
  const comparison = {
    structured: value.structured === true,
    code: safeIdentifier(value.code, ""),
    retryable: value.retryable === true,
  }
  return comparison.structured && comparison.code
    ? passed(comparison)
    : failed("error_result_invalid", comparison)
}

function probeShutdownResults(value) {
  const results = {}
  for (const trigger of MANDATORY_SHUTDOWN_TRIGGERS) {
    const entry = value && typeof value === "object" ? value[trigger] : null
    const field = "shutdown_" + trigger
    if (!entry || typeof entry !== "object") {
      results[field] = missing("shutdown_trigger_not_configured")
      continue
    }
    const comparison = {
      trigger,
      observed: entry.observed === true,
      outcome: safeIdentifier(entry.outcome, ""),
    }
    results[field] = comparison.observed && comparison.outcome
      ? passed(comparison)
      : failed("shutdown_trigger_result_invalid", comparison)
  }
  return results
}

async function probeExactCwd(expectedCwd, workspaceIdentity) {
  if (!expectedCwd || !workspaceIdentity) return missing("exact_cwd_contract_not_configured")
  try {
    const actual = await realpath(process.cwd())
    const expected = await realpath(expectedCwd)
    const comparison = {
      logical_workspace: workspaceIdentity,
      exact_match: actual === expected,
    }
    return actual === expected
      ? passed(comparison, { actual_path_hash: sha256Text(actual), expected_path_hash: sha256Text(expected) })
      : failed("exact_cwd_mismatch", comparison, { actual_path_hash: sha256Text(actual), expected_path_hash: sha256Text(expected) })
  } catch (error) {
    return failed(safeReason(error))
  }
}

async function captureRuntimeSnapshot(options) {
  if (process.platform !== "linux") throw new Error("linux_required")
  const signingKey = options.signingKey
  if (!validSigningKey(signingKey)) throw new Error("signing_key_missing")
  const scratchRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-path1-parity-capture-"))
  const cleanupFailures = []
  const workspaceIdentity = options.workspaceIdentity
  const results = {}
  try {
    results.exact_cwd = await probeExactCwd(options.expectedCwd, workspaceIdentity)
    results.arbitrary_accessible_directory = await probeWritableDirectory(
      options.arbitraryDirectory,
      "arbitrary-accessible",
      scratchRoot,
    )
    results.home_access = await probeWritableDirectory(options.homeProbePath, "home", scratchRoot)
    results.tmp_access = await probeWritableDirectory(options.tmpProbePath, "tmp", scratchRoot)
    results.post_enrollment_repository = await probeRepository(options.repositoryParent, scratchRoot)
    results.git_file_terminal = results.post_enrollment_repository.status === "passed"
      ? passed({ git: true, file: true, terminal: true })
      : failed("repository_boundary_not_available")

    const chain = await processChain()
    const markerSet = await environmentMarkerSet(chain)
    results.provider_ancestry = await probeProvider(options.topology, options.provider, options.workerFresh)
    results.managed_isolation_environment = await probeIsolationEnvironment(markerSet)
    results.mount_visibility = await probeMountVisibility({
      home: options.homeProbePath,
      tmp: options.tmpProbePath,
      workspace: options.expectedCwd,
      repository: options.repositoryParent,
    })
    results.privilege_state = await probePrivilegeState()
    results.network_reachability = await probeNetwork(options.networkProbe)
    results.package_tool_installation = await probePackageTool(options.packageProbe)
    results.official_provider_identity = probeProviderIdentity(
      options.provider,
      options.providerVersion,
      options.providerIdentity,
    )
    results.reconnect_history_result_identity = probeReconnectionHistory(options.reconnectHistory)
    results.errors = probeErrors(options.errorResult)

    const shutdownResults = probeShutdownResults(options.shutdownResults)
    Object.assign(results, shutdownResults)
  } finally {
    await rm(scratchRoot, { recursive: true, force: false }).catch(() => cleanupFailures.push("scratch_root"))
  }

  const externalCleanup = options.cleanupResult
  if (!externalCleanup || typeof externalCleanup !== "object" || externalCleanup.__invalid) {
    results.cleanup = cleanupFailures.length === 0
      ? missing("cleanup_result_not_configured")
      : failed("cleanup_failed")
  } else {
    const comparison = {
      temporary_data_removed: externalCleanup.temporary_data_removed === true,
      control_state_separate: externalCleanup.control_state_separate === true,
      no_failure: externalCleanup.no_failure === true,
    }
    results.cleanup = cleanupFailures.length === 0
      && Object.values(comparison).every(Boolean)
      ? passed(comparison)
      : failed("cleanup_failed", comparison)
  }

  return createSignedSnapshot({
    reviewedCommit: options.reviewedCommit,
    buildId: options.buildId,
    topology: options.topology,
    workerFresh: options.workerFresh,
    provider: {
      name: options.provider,
      version: options.providerVersion,
      executable: options.providerIdentity?.executable ?? options.provider,
      official: options.providerIdentity?.official === true,
    },
    capture: {
      boundary: options.boundary,
      runtimeProbe: true,
      invokedInsideProviderTurn: options.invokedInsideProviderTurn,
      source_pid: process.pid,
      fixture: false,
    },
    results,
    signingKey,
  })
}

function readSigningKeyFromEnvironment(name = "CHARIOX_PARITY_SIGNING_KEY") {
  const raw = process.env[name]
  if (!raw) return null
  if (raw.startsWith("base64:")) return Buffer.from(raw.slice("base64:".length), "base64")
  if (raw.startsWith("hex:")) return Buffer.from(raw.slice("hex:".length), "hex")
  return Buffer.from(raw, "utf8")
}

function readRequiredValue(argv, index, flag) {
  const value = argv[index]
  if (!value || value.startsWith("--")) throw new Error(flag + "_value_missing")
  return value
}

function parseArguments(argv) {
  const mode = argv[2] === "--help" || argv[2] === "-h" ? "help" : (argv[2] ?? "help")
  const options = {
    mode,
    output: null,
    ordinary: null,
    path1: null,
    report: null,
    topology: process.env.CHARIOX_PARITY_TOPOLOGY ?? null,
    reviewedCommit: process.env.CHARIOX_PARITY_REVIEWED_COMMIT ?? null,
    buildId: process.env.CHARIOX_PARITY_BUILD_ID ?? null,
    provider: process.env.CHARIOX_PARITY_PROVIDER ?? null,
    providerVersion: process.env.CHARIOX_PARITY_PROVIDER_VERSION ?? null,
    providerIdentity: parseJsonEnv("CHARIOX_PARITY_PROVIDER_IDENTITY_JSON"),
    boundary: process.env.CHARIOX_PARITY_BOUNDARY ?? null,
    workerFresh: parseBoolean(process.env.CHARIOX_PARITY_FRESH_WORKER),
    invokedInsideProviderTurn: parseBoolean(process.env.CHARIOX_PARITY_IN_PROVIDER_TURN),
    signingKey: readSigningKeyFromEnvironment(),
    signingKeyEnv: "CHARIOX_PARITY_SIGNING_KEY",
    expectedCwd: process.env.CHARIOX_PARITY_EXPECTED_CWD ?? null,
    workspaceIdentity: process.env.CHARIOX_PARITY_WORKSPACE_ID ?? null,
    arbitraryDirectory: process.env.CHARIOX_PARITY_ARBITRARY_DIR ?? null,
    homeProbePath: process.env.CHARIOX_PARITY_HOME_PROBE_PATH ?? "/home/chariox",
    tmpProbePath: process.env.CHARIOX_PARITY_TMP_PROBE_PATH ?? "/tmp",
    repositoryParent: process.env.CHARIOX_PARITY_REPOSITORY_PARENT ?? null,
    networkProbe: parseJsonEnv("CHARIOX_PARITY_NETWORK_PROBE_JSON"),
    packageProbe: parseJsonEnv("CHARIOX_PARITY_PACKAGE_PROBE_JSON"),
    reconnectHistory: parseJsonEnv("CHARIOX_PARITY_RECONNECT_HISTORY_JSON"),
    errorResult: parseJsonEnv("CHARIOX_PARITY_ERROR_RESULT_JSON"),
    cleanupResult: parseJsonEnv("CHARIOX_PARITY_CLEANUP_RESULT_JSON"),
    shutdownResults: parseJsonEnv("CHARIOX_PARITY_SHUTDOWN_RESULTS_JSON"),
    allowFixture: false,
  }
  for (let index = 3; index < argv.length; index += 1) {
    const flag = argv[index]
    if (flag === "--output") options.output = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--ordinary") options.ordinary = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--path1") options.path1 = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--report") options.report = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--topology") options.topology = readRequiredValue(argv, ++index, flag)
    else if (flag === "--reviewed-commit") options.reviewedCommit = readRequiredValue(argv, ++index, flag)
    else if (flag === "--build-id") options.buildId = readRequiredValue(argv, ++index, flag)
    else if (flag === "--provider") options.provider = readRequiredValue(argv, ++index, flag)
    else if (flag === "--provider-version") options.providerVersion = readRequiredValue(argv, ++index, flag)
    else if (flag === "--boundary") options.boundary = readRequiredValue(argv, ++index, flag)
    else if (flag === "--expected-cwd") options.expectedCwd = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--workspace-id") options.workspaceIdentity = readRequiredValue(argv, ++index, flag)
    else if (flag === "--arbitrary-dir") options.arbitraryDirectory = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--home-probe-path") options.homeProbePath = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--tmp-probe-path") options.tmpProbePath = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--repository-parent") options.repositoryParent = path.resolve(readRequiredValue(argv, ++index, flag))
    else if (flag === "--signing-key-env") {
      options.signingKeyEnv = readRequiredValue(argv, ++index, flag)
      options.signingKey = readSigningKeyFromEnvironment(options.signingKeyEnv)
    } else if (flag === "--fresh-worker") options.workerFresh = true
    else if (flag === "--inside-provider-turn") options.invokedInsideProviderTurn = true
    else if (flag === "--allow-fixture") options.allowFixture = true
    else if (flag === "--help" || flag === "-h") options.mode = "help"
    else throw new Error("unknown_argument")
  }
  return options
}

function printHelp() {
  console.log([
    "Usage:",
    "  node scripts/managed-path1-provider-parity-drill.mjs capture --topology ordinary|path1 --output snapshot.json",
    "  node scripts/managed-path1-provider-parity-drill.mjs compare --ordinary ordinary.json --path1 path1.json --report report.json",
    "",
    "Capture is invoked inside an official provider turn or approved remote-command boundary.",
    "The signing key is read from CHARIOX_PARITY_SIGNING_KEY and is never printed.",
    "Capture requires runtime probes; source/static assertions cannot produce an accepted snapshot.",
  ].join("\n"))
}

async function captureCommand(options) {
  if (!options.output) throw new Error("capture_output_missing")
  if (!validTopology(options.topology)) throw new Error("capture_topology_invalid")
  if (!validCommit(options.reviewedCommit)) throw new Error("capture_reviewed_commit_invalid")
  if (!validBuildId(options.buildId)) throw new Error("capture_build_id_invalid")
  if (!OFFICIAL_PROVIDERS.includes(options.provider)) throw new Error("capture_provider_invalid")
  if (!options.providerVersion) throw new Error("capture_provider_version_missing")
  if (!validBoundary(options.boundary)) throw new Error("capture_boundary_invalid")
  if (options.topology === "path1" && !options.workerFresh) {
    throw new Error("capture_fresh_worker_required")
  }
  if (!options.invokedInsideProviderTurn) throw new Error("capture_provider_turn_boundary_required")
  const snapshot = await captureRuntimeSnapshot(options)
  await mkdir(path.dirname(options.output), { recursive: true })
  await writeFile(options.output, JSON.stringify(snapshot, null, 2) + "\n", { encoding: "utf8", mode: 0o600 })
  const failedRows = REQUIRED_RESULT_FIELDS.filter((field) => snapshot.results[field]?.status !== "passed")
  return {
    ok: failedRows.length === 0,
    mode: "capture",
    output: options.output,
    failed_rows: failedRows,
    result_count: REQUIRED_RESULT_FIELDS.length,
  }
}

async function compareCommand(options) {
  if (!options.ordinary || !options.path1) throw new Error("compare_snapshot_missing")
  if (!validCommit(options.reviewedCommit)) throw new Error("compare_reviewed_commit_invalid")
  if (!validBuildId(options.buildId)) throw new Error("compare_build_id_invalid")
  if (!validSigningKey(options.signingKey)) throw new Error("compare_signing_key_missing")
  let ordinary
  let path1
  try {
    ordinary = JSON.parse(await readFile(options.ordinary, "utf8"))
    path1 = JSON.parse(await readFile(options.path1, "utf8"))
  } catch {
    throw new Error("compare_snapshot_unreadable")
  }
  const comparison = compareSnapshots(ordinary, path1, {
    expectedReviewedCommit: options.reviewedCommit,
    expectedBuildId: options.buildId,
    signingKey: options.signingKey,
    allowFixture: options.allowFixture,
  })
  const report = {
    schema: SNAPSHOT_SCHEMA,
    ok: comparison.ok,
    compared_fields: comparison.comparedFields,
    errors: comparison.errors,
  }
  if (options.report) {
    await mkdir(path.dirname(options.report), { recursive: true })
    await writeFile(options.report, JSON.stringify(report, null, 2) + "\n", { encoding: "utf8", mode: 0o600 })
  }
  return {
    ...report,
    mode: "compare",
    report: options.report,
  }
}

export async function runCli(argv = process.argv) {
  const options = parseArguments(argv)
  if (options.mode === "help") {
    printHelp()
    return { ok: true, mode: "help" }
  }
  if (options.mode === "capture") return await captureCommand(options)
  if (options.mode === "compare") return await compareCommand(options)
  throw new Error("unknown_mode")
}

function isMain() {
  const entry = process.argv[1]
  return entry && path.resolve(entry) === path.resolve(new URL(import.meta.url).pathname)
}

if (isMain()) {
  runCli().then((summary) => {
    console.log(JSON.stringify(summary))
    if (summary.ok !== true) process.exitCode = 1
  }).catch((error) => {
    console.error(JSON.stringify({ ok: false, error: safeReason(error) }))
    process.exitCode = 1
  })
}

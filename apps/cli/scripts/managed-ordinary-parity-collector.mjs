#!/usr/bin/env node

import { execFile } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdir, readFile, realpath, writeFile } from "node:fs/promises"
import { basename, dirname, isAbsolute, join, resolve } from "node:path"

import {
  ALLOWED_CAPTURE_BOUNDARIES,
  ALLOWED_TOPOLOGIES,
  MATRIX_SCHEMA,
  ROW_DEFINITIONS,
  SHUTDOWN_EXPECTATIONS,
  createSignedManifest,
  validateManifest,
} from "./managed-ordinary-parity-matrix.mjs"

export const DEFAULT_TIMEOUT_MS = 45_000
export const OFFICIAL_PROVIDERS = Object.freeze(["claude", "codex", "opencode"])
export const COLLECTOR_CHECKS = Object.freeze(
  Object.fromEntries(ROW_DEFINITIONS.map(({ id, checks }) => [id, [...checks]])),
)

const REVIEWED_COMMIT = /^[0-9a-f]{40}$/i
const PROTOCOL_VERSION = /^\d+$/
const DIGEST = /^sha256:[0-9a-f]{64}$/i
const PROBE_RELATIVE_PATH = "apps/cli/scripts/managed-ordinary-parity-probe.mjs"
const RELEASE_VERIFIER_RELATIVE_PATH = "deploy/managed-kernel/verify-image-release.mjs"
const NODE_FILESYSTEM = Object.freeze({ mkdir, readFile, realpath, writeFile })

class CollectorError extends Error {
  constructor(code, message, details = {}) {
    super(message)
    this.name = "ParityCollectorError"
    this.code = code
    Object.assign(this, details)
  }
}

export { CollectorError }

function isPlainObject(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function sha256(value) {
  return createHash("sha256").update(String(value)).digest("hex")
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])]))
}

function redactText(value) {
  return String(value)
    .replace(/("?(?:api[_-]?key|authorization|credential|password|secret|token)"?)\s*:\s*"[^"]*"/gi, '$1:"<redacted>"')
    .replace(/((?:api[_-]?key|authorization|credential|password|secret|token))\s*[:=]\s*[^\s,;]+/gi, "$1=<redacted>")
    .replace(/Bearer\s+[^\s,;]+/gi, "Bearer <redacted>")
}

function redactValue(value, key = "") {
  if (/api[_-]?key|authorization|credential|password|secret|token/i.test(key)) return "<redacted>"
  if (Array.isArray(value)) return value.map((entry) => redactValue(entry))
  if (!value || typeof value !== "object") return typeof value === "string" ? redactText(value) : value
  return Object.fromEntries(Object.entries(value).map(([entryKey, entryValue]) => [
    entryKey,
    redactValue(entryValue, entryKey),
  ]))
}

function nowIso(clock) {
  const value = typeof clock === "function" ? clock() : new Date()
  const date = value instanceof Date ? value : new Date(value)
  if (Number.isNaN(date.getTime())) throw new CollectorError("clock_invalid", "clock returned an invalid timestamp")
  return date.toISOString()
}

function commandText(command, args) {
  return JSON.stringify([String(command), ...args.map((arg) => String(arg))])
}

function resultPayload(stdout, rowId, checkId) {
  const text = String(stdout ?? "").trim()
  if (!text) throw new CollectorError("malformed_output", `${rowId}/${checkId} produced empty stdout`)
  let payload
  try {
    payload = JSON.parse(text)
  } catch (error) {
    throw new CollectorError("malformed_output", `${rowId}/${checkId} produced non-JSON stdout`, { cause: error })
  }
  if (!isPlainObject(payload) || payload.ok !== true) {
    throw new CollectorError("probe_not_accepted", `${rowId}/${checkId} did not report an observed successful probe`)
  }
  const result = Object.hasOwn(payload, "result") ? payload.result : payload
  if (!isPlainObject(result)) {
    throw new CollectorError("malformed_output", `${rowId}/${checkId} result is not an object`)
  }
  return result
}

function normalizeCommandResult(result, error = null) {
  const raw = result ?? {}
  const code = Number.isInteger(raw.code)
    ? raw.code
    : Number.isInteger(error?.code)
      ? error.code
      : null
  const signal = raw.signal ?? error?.signal ?? null
  const killed = raw.killed === true || error?.killed === true
  const timedOut = raw.timedOut === true
    || raw.timed_out === true
    || raw.code === "ETIMEDOUT"
    || error?.code === "ETIMEDOUT"
    || error?.timedOut === true
    || error?.timed_out === true
  return {
    code,
    signal,
    killed,
    timedOut,
    stdout: String(raw.stdout ?? error?.stdout ?? ""),
    stderr: String(raw.stderr ?? error?.stderr ?? error?.message ?? ""),
  }
}

export async function defaultRunCommand(command, args = [], options = {}) {
  const timeout = options.timeout
  const childOptions = {
    encoding: "utf8",
    maxBuffer: 4 * 1024 * 1024,
    ...options,
  }
  delete childOptions.timeout
  return new Promise((resolvePromise, rejectPromise) => {
    let timedOut = false
    let timer = null
    const child = execFile(command, args, childOptions, (error, stdout, stderr) => {
      if (timer) clearTimeout(timer)
      if (error) {
        if (timedOut) error.timedOut = true
        error.stdout = stdout ?? ""
        error.stderr = stderr ?? ""
        rejectPromise(error)
        return
      }
      resolvePromise({ code: 0, signal: null, killed: false, timedOut: false, stdout: stdout ?? "", stderr: stderr ?? "" })
    })
    if (Number.isFinite(timeout) && timeout > 0) {
      timer = setTimeout(() => {
        timedOut = true
        child.kill(options.killSignal ?? "SIGTERM")
      }, timeout)
      timer.unref?.()
    }
  })
}

function expectedBoolean(result, key, rowId, checkId, expected = true) {
  if (result[key] !== expected) {
    throw new CollectorError(
      key === "cwd_matches_requested" ? "cwd_mismatch" : "probe_assertion_failed",
      `${rowId}/${checkId} expected ${key}=${String(expected)}`,
      { rowId, checkId, key },
    )
  }
}

function requireObserved(result, rowId, checkId) {
  if (result.observed !== true) {
    throw new CollectorError("probe_assertion_failed", `${rowId}/${checkId} did not provide observed=true`, {
      rowId,
      checkId,
    })
  }
}

const GENERIC_REQUIREMENTS = Object.freeze({
  "MP-01/mount_visibility": ["mounts_match_ordinary", "mount_probe_complete"],
  "MP-01/privilege_state": ["capabilities_match_ordinary", "umask_matches_ordinary"],
  "MP-01/network_reachability": ["network_matches_ordinary", "address_families_recorded"],
  "MP-01/package_tool_installation": ["tool_probe_succeeded", "install_probe_succeeded"],
  "MP-02/directory_discovery": ["exact_path_accessible"],
  "MP-02/exact_path_entry": ["exact_path_accessible", "cwd_matches_requested"],
  "MP-02/directory_creation": ["created_and_accessible"],
  "MP-02/home_access": ["accessible"],
  "MP-02/tmp_access": ["accessible"],
  "MP-03/control_file_protection": ["control_file_denied", "sibling_accessible"],
  "MP-03/filesystem_permissions": ["permissions_match_ordinary"],
  "MP-04/provider_environment": ["home_matches_ordinary", "chariox_home_matches_ordinary", "cwd_matches_requested", "ordinary_user"],
  "MP-05/empty_workspace": ["workspace_created", "control_state_separate"],
  "MP-05/copied_repository": ["repository_accessible"],
  "MP-05/repository_basename": ["source_basename_preserved"],
  "MP-05/basename_collision": ["collision_rejected"],
  "MP-05/worktree_placement": ["worktree_user_path", "control_root_not_workspace"],
  "MP-06/repository_root_default": ["default_root_correct"],
  "MP-06/repository_root_custom": ["custom_root_persisted"],
  "MP-06/repository_root_inheritance": ["child_inherits_root"],
  "MP-06/repository_root_override_rejected": ["client_override_rejected"],
  "MP-08/session_agent_launch": ["session_created", "agent_created", "official_command"],
  "MP-08/terminal_file_git": ["terminal_ok", "file_ok", "git_ok"],
  "MP-08/attachments_permissions_capabilities": ["attachments_ok", "permissions_ok", "capabilities_ok"],
  "MP-08/project_setup": ["project_setup_ok"],
  "MP-08/reconnect_orphan_recovery": ["reconnect_ok", "orphan_recovered"],
  "MP-08/restart_recovery": ["restart_recovered"],
  "MP-08/reconnect_history_result_identity": ["history_preserved", "result_identity_preserved"],
  "MP-08/queued_prompts": ["queued_prompt_preserved", "queued_prompt_advanced"],
  "MP-08/active_turn_state": ["active_turn_state_preserved"],
  "MP-08/resource_limits": ["limits_observed"],
  "MP-08/structured_errors": ["structured_errors"],
  "MP-08/protocol_behavior": ["protocol_behavior_ok"],
  "MP-08/cleanup": ["owned_processes_gone", "owned_artifacts_removed", "foreign_processes_untouched", "cleanup_complete"],
})

function normalizeGenericResult(result, rowId, checkId) {
  requireObserved(result, rowId, checkId)
  const required = GENERIC_REQUIREMENTS[`${rowId}/${checkId}`] ?? []
  for (const key of required) {
    expectedBoolean(result, key, rowId, checkId, true)
  }
  if (rowId === "MP-03" && checkId === "control_file_protection") {
    expectedBoolean(result, "parent_workspace_accessible", rowId, checkId, true)
  }
  const normalized = { observed: true }
  for (const key of required) normalized[key] = result[key]
  if (rowId === "MP-01" && checkId === "privilege_state") {
    expectedBoolean(result, "no_new_privs", rowId, checkId, false)
    expectedBoolean(result, "seccomp_matches_ordinary", rowId, checkId, true)
    normalized.no_new_privs = false
  }
  if (rowId === "MP-02" && checkId === "directory_discovery") {
    normalized.child_enumeration_denied = result.child_enumeration_denied === true
  }
  return normalized
}

function normalizeProviderAncestry(result) {
  requireObserved(result, "MP-01", "provider_ancestry")
  expectedBoolean(result, "provider_observed", "MP-01", "provider_ancestry")
  expectedBoolean(result, "bwrap_ancestor", "MP-01", "provider_ancestry", false)
  expectedBoolean(result, "fresh_worker", "MP-01", "provider_ancestry")
  expectedBoolean(result, "ancestry_complete", "MP-01", "provider_ancestry")
  return { observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true }
}

function normalizeManagedIsolation(result) {
  requireObserved(result, "MP-01", "managed_isolation_environment")
  expectedBoolean(result, "managed_marker_absent", "MP-01", "managed_isolation_environment")
  expectedBoolean(result, "bwrap_environment_absent", "MP-01", "managed_isolation_environment")
  return { observed: true, managed_marker_absent: true, bwrap_environment_absent: true }
}

function normalizeReleaseResult(result, topology) {
  requireObserved(result, "MP-07", "signed_release_activation")
  if (topology === "ordinary") {
    expectedBoolean(result, "ordinary_release_not_applicable", "MP-07", "signed_release_activation")
    expectedBoolean(result, "signed_release_present", "MP-07", "signed_release_activation", false)
    expectedBoolean(result, "signature_verified", "MP-07", "signed_release_activation", false)
    expectedBoolean(result, "atomic_activation", "MP-07", "signed_release_activation", false)
    expectedBoolean(result, "rollback_verified", "MP-07", "signed_release_activation", false)
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
  expectedBoolean(result, "signed_release_present", "MP-07", "signed_release_activation")
  expectedBoolean(result, "signature_verified", "MP-07", "signed_release_activation")
  expectedBoolean(result, "atomic_activation", "MP-07", "signed_release_activation")
  expectedBoolean(result, "rollback_verified", "MP-07", "signed_release_activation")
  if (!DIGEST.test(result.release_digest ?? "")) {
    throw new CollectorError("probe_assertion_failed", "managed release digest is not a sha256 digest")
  }
  return {
    exemption: "managed-signed-release",
    observed: "managed-signed-release",
    signed_release: true,
    signature_verified: true,
    atomic_activation: true,
    rollback_verified: true,
    release_digest: result.release_digest,
  }
}

function normalizeShutdownResult(result, topology, checkId) {
  requireObserved(result, "MP-09", checkId)
  const trigger = checkId.slice("shutdown_".length)
  if (result.trigger !== trigger) {
    throw new CollectorError("shutdown_trigger_mismatch", `${checkId} reported ${String(result.trigger)}`)
  }
  const expectation = SHUTDOWN_EXPECTATIONS[trigger]
  if (!expectation) throw new CollectorError("shutdown_trigger_unsupported", `${checkId} has no reviewed expectation`)
  expectedBoolean(result, "observation_complete", "MP-09", checkId)
  expectedBoolean(result, "managed_policy", "MP-09", checkId, topology === "path1")
  const ordinary = topology === "ordinary"
  const expectedOutcome = ordinary ? "ordinary-remained-running" : expectation.outcome
  if (result.expected_outcome !== expectedOutcome || result.observed_outcome !== expectedOutcome) {
    throw new CollectorError("shutdown_outcome_mismatch", `${checkId} did not observe ${expectedOutcome}`)
  }
  const workerStopped = ordinary ? false : expectation.workerStopped
  const cleanupConfirmed = ordinary ? false : expectation.cleanupConfirmed
  const measuredFromLastAgentFinished = ordinary ? false : expectation.measuredFromLastAgentFinished
  expectedBoolean(result, "worker_stopped", "MP-09", checkId, workerStopped)
  expectedBoolean(result, "cleanup_confirmed", "MP-09", checkId, cleanupConfirmed)
  expectedBoolean(result, "measured_from_last_agent_finished", "MP-09", checkId, measuredFromLastAgentFinished)
  const configuredDelay = result.configured_delay_seconds ?? null
  const observedDelay = result.observed_delay_seconds ?? null
  if (ordinary) {
    if (configuredDelay !== null || observedDelay !== null) {
      throw new CollectorError("shutdown_delay_invalid", `${checkId} ordinary control must not report a managed delay`)
    }
  } else if (expectation.delay === null) {
    if (configuredDelay !== null || observedDelay !== null) {
      throw new CollectorError("shutdown_delay_invalid", `${checkId} must report null delays`)
    }
  } else {
    const validConfigured = Number.isInteger(configuredDelay) && configuredDelay >= 0
      && (expectation.delay === "positive" ? configuredDelay > 0 : configuredDelay === expectation.delay)
    if (!validConfigured) throw new CollectorError("shutdown_delay_invalid", `${checkId} configured delay is invalid`)
    if (expectation.observesDelay) {
      if (!Number.isFinite(observedDelay) || observedDelay < configuredDelay || observedDelay > configuredDelay + 120) {
        throw new CollectorError("shutdown_delay_invalid", `${checkId} observed delay is outside the reviewed tolerance`)
      }
    } else if (observedDelay !== null) {
      throw new CollectorError("shutdown_delay_invalid", `${checkId} must not report an elapsed delay`)
    }
  }
  return {
    exemption: ordinary ? "ordinary-no-managed-shutdown" : "managed-auto-shutdown",
    trigger,
    expected_outcome: expectedOutcome,
    observed_outcome: expectedOutcome,
    managed_policy: !ordinary,
    observation_complete: true,
    worker_stopped: workerStopped,
    cleanup_confirmed: cleanupConfirmed,
    measured_from_last_agent_finished: measuredFromLastAgentFinished,
    configured_delay_seconds: configuredDelay,
    observed_delay_seconds: observedDelay,
  }
}

function normalizeBoundary(result, expectedBoundary) {
  requireObserved(result, "MP-10", "capture_boundary")
  if (result.boundary !== expectedBoundary) {
    throw new CollectorError("capture_boundary_mismatch", `probe reported ${String(result.boundary)}`)
  }
  expectedBoolean(result, "inside_provider_turn", "MP-10", "capture_boundary")
  expectedBoolean(result, "independent", "MP-10", "capture_boundary")
  return { observed: true, boundary_verified: true, inside_provider_turn: true, independent: true }
}

function normalizeFreshWorker(result) {
  requireObserved(result, "MP-10", "fresh_worker")
  expectedBoolean(result, "fresh_worker", "MP-10", "fresh_worker")
  if (!nonEmptyString(result.worker_id)) {
    throw new CollectorError("fresh_worker_identity_missing", "fresh worker probe did not return a worker identity")
  }
  return { observed: true, fresh_worker: true, worker_identity_observed: true }
}

function normalizeProviderIdentity(result, provider, executable) {
  requireObserved(result, "MP-08", "official_provider_identity")
  expectedBoolean(result, "official", "MP-08", "official_provider_identity")
  if (result.provider_name !== provider) {
    throw new CollectorError("provider_identity_mismatch", `provider probe reported ${String(result.provider_name)}`)
  }
  expectedBoolean(result, "executable_matches", "MP-08", "official_provider_identity")
  return { observed: true, official: true, provider_name: provider, executable_matches: true, executable_basename: executable }
}

function parseInteger(text, label) {
  const value = String(text).trim()
  if (!PROTOCOL_VERSION.test(value)) throw new CollectorError("protocol_identity_invalid", `${label} is not an integer`)
  return Number(value)
}

function parseRelayVersion(text) {
  const match = String(text).match(/RELAY_PEER_PROTOCOL_VERSION\s*:\s*u\d+\s*=\s*(\d+)/)
  return parseInteger(match?.[1] ?? text, "relay protocol")
}

function parseRepoBlob(text, relativePath, label) {
  const records = String(text).trim().split("\n").filter(Boolean)
  if (records.length !== 1) throw new CollectorError("repo_identity_invalid", `${label} must resolve to one tracked file`)
  const match = /^(100644|100755) blob ([0-9a-f]{40})\t(.+)$/.exec(records[0])
  if (!match || match[3] !== relativePath) {
    throw new CollectorError("repo_identity_invalid", `${label} is not the expected tracked file`)
  }
  return { mode: match[1], gitBlob: match[2] }
}

function parseReleaseManifest(bytes, expected) {
  let manifest
  try {
    manifest = JSON.parse(Buffer.from(bytes).toString("utf8"))
  } catch (error) {
    throw new CollectorError("kernel_release_invalid", "signed kernel release manifest is invalid JSON", { cause: error })
  }
  if (!isPlainObject(manifest)
    || manifest.schemaVersion !== 2
    || manifest.sourceCommit !== expected.reviewedCommit
    || manifest.sourceTree !== expected.sourceTree
    || !Array.isArray(manifest.artifacts)) {
    throw new CollectorError("kernel_release_identity_mismatch", "signed kernel release is not bound to the reviewed commit")
  }
  const artifact = manifest.artifacts.find((entry) => entry?.name === "chariox-kernel")
  if (!isPlainObject(artifact)
    || artifact.path !== "/usr/local/bin/chariox-kernel"
    || !DIGEST.test(artifact.sha256 ?? "")) {
    throw new CollectorError("kernel_release_kernel_missing", "signed kernel release does not declare chariox-kernel")
  }
  return {
    sourceCommit: manifest.sourceCommit,
    sourceTree: manifest.sourceTree,
    kernelDigest: artifact.sha256,
  }
}

async function verifyRepoFile(ctx, runText, git, relativePath, step) {
  const treeStep = await runText(ctx, "MP-10", "source_protocol_identity", `${step}-tree`, git, ["ls-tree", "-r", "--full-tree", ctx.reviewedCommit, "--", relativePath], { cwd: ctx.sourceRoot })
  const tree = parseRepoBlob(treeStep.text, relativePath, step)
  const hashStep = await runText(ctx, "MP-10", "source_protocol_identity", `${step}-hash`, git, ["hash-object", "--", relativePath], { cwd: ctx.sourceRoot })
  if (hashStep.text !== tree.gitBlob) {
    throw new CollectorError("repo_identity_mismatch", `${step} does not match the reviewed commit`)
  }
  return {
    gitBlob: tree.gitBlob,
    mode: tree.mode,
    evidenceRefs: [treeStep.stepResult.evidencePath, hashStep.stepResult.evidencePath],
  }
}

function validateOptions(options, processApi) {
  if (processApi.platform !== "linux") throw new CollectorError("unsupported_platform", "parity collection requires Linux")
  if (!ALLOWED_TOPOLOGIES.includes(options.topology)) throw new CollectorError("topology_invalid", "topology must be ordinary or path1")
  if (!ALLOWED_CAPTURE_BOUNDARIES.includes(options.boundary)) throw new CollectorError("capture_boundary_invalid", "unsupported capture boundary")
  if (!REVIEWED_COMMIT.test(options.reviewedCommit ?? "")) throw new CollectorError("reviewed_commit_invalid", "reviewed commit must be a 40-hex SHA")
  if (!nonEmptyString(options.buildId)) throw new CollectorError("build_id_missing", "expected build id is required")
  if (!Number.isInteger(options.kernelProtocol) || options.kernelProtocol < 1) throw new CollectorError("kernel_protocol_invalid", "expected kernel protocol is required")
  if (!Number.isInteger(options.relayProtocol) || options.relayProtocol < 1) throw new CollectorError("relay_protocol_invalid", "expected relay protocol is required")
  if (!OFFICIAL_PROVIDERS.includes(options.provider)) throw new CollectorError("provider_invalid", "provider is not an official provider")
  if (Object.hasOwn(options, "probeCommand") || Object.hasOwn(options, "probeArgs")) {
    throw new CollectorError("probe_command_unsupported", "the parity probe is repo-owned and cannot be overridden")
  }
  for (const [key, label] of [["providerCommand", "provider command"], ["kernelBinary", "kernel binary"], ["kernelReleaseRoot", "kernel release root"], ["kernelReleaseDigest", "kernel release digest"], ["kernelReleasePublicKey", "kernel release public key"], ["outputPath", "output path"]]) {
    if (!nonEmptyString(options[key])) throw new CollectorError("tool_missing", `${label} is required`)
  }
  for (const [key, label] of [["kernelBinary", "kernel binary"], ["kernelReleaseRoot", "kernel release root"], ["kernelReleasePublicKey", "kernel release public key"]]) {
    if (!isAbsolute(options[key])) throw new CollectorError("path_invalid", `${label} must be an absolute path`)
  }
  if (options.topology === "path1" && !nonEmptyString(options.kernelBuilderPublicKey)) {
    throw new CollectorError("builder_key_missing", "Path-1 capture requires an independently supplied trusted builder public key")
  }
  if (options.kernelBuilderPublicKey && !isAbsolute(options.kernelBuilderPublicKey)) {
    throw new CollectorError("path_invalid", "trusted builder public key must be an absolute path")
  }
  if (options.expectedCwd !== undefined && !isAbsolute(options.expectedCwd)) {
    throw new CollectorError("path_invalid", "expected provider working directory must be absolute")
  }
  if (!DIGEST.test(options.kernelReleaseDigest)) throw new CollectorError("kernel_release_digest_invalid", "kernel release digest must be a sha256 digest")
  if (!options.signingKey || (typeof options.signingKey !== "string" && !Buffer.isBuffer(options.signingKey))) {
    throw new CollectorError("signing_key_missing", "a signing key is required")
  }
  if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1) throw new CollectorError("timeout_invalid", "timeout must be positive")
}

function createDefaultFilesystem() {
  return NODE_FILESYSTEM
}

export function createParityCollector({
  filesystem = createDefaultFilesystem(),
  runCommand = defaultRunCommand,
  clock = () => new Date(),
  processApi = process,
} = {}) {
  if (!filesystem || typeof filesystem.mkdir !== "function" || typeof filesystem.readFile !== "function" || typeof filesystem.realpath !== "function" || typeof filesystem.writeFile !== "function") {
    throw new TypeError("filesystem must provide mkdir, readFile, realpath, and writeFile")
  }
  if (typeof runCommand !== "function") throw new TypeError("runCommand must be a function")

  async function executeStep(ctx, rowId, checkId, step, command, args, options = {}) {
    const startedAt = nowIso(clock)
    let raw
    let caught = null
    try {
      raw = await runCommand(command, args, {
        cwd: options.cwd ?? ctx.sourceRoot,
        env: options.env,
        timeout: ctx.timeoutMs,
        maxBuffer: 4 * 1024 * 1024,
      })
    } catch (error) {
      caught = error
    }
    const normalized = normalizeCommandResult(raw, caught)
    const finishedAt = nowIso(clock)
    const record = {
      captured_at: finishedAt,
      started_at: startedAt,
      finished_at: finishedAt,
      topology: ctx.topology,
      row_id: rowId,
      check_id: checkId,
      step,
      command,
      args: args.map((arg) => String(arg)),
      cwd: options.cwd ?? ctx.sourceRoot,
      exit_code: normalized.code,
      signal: normalized.signal,
      killed: normalized.killed,
      timed_out: normalized.timedOut,
      stdout_sha256: `sha256:${sha256(normalized.stdout)}`,
      stderr_sha256: `sha256:${sha256(normalized.stderr)}`,
      stdout_redacted: redactText(normalized.stdout),
      stderr_redacted: redactText(normalized.stderr),
      error_code: typeof caught?.code === "string" ? caught.code : null,
    }
    const evidencePath = join(ctx.evidenceDir, ctx.topology, rowId, checkId, `${step}.json`)
    await filesystem.mkdir(dirname(evidencePath), { recursive: true })
    await filesystem.writeFile(evidencePath, `${JSON.stringify(redactValue(record), null, 2)}\n`, "utf8")
    return { ...normalized, evidencePath, commandText: commandText(command, args), record }
  }

  async function runText(ctx, rowId, checkId, step, command, args, { allowEmpty = false, cwd } = {}) {
    const stepResult = await executeStep(ctx, rowId, checkId, step, command, args, { cwd })
    if (stepResult.timedOut) throw new CollectorError("command_timeout", `${rowId}/${checkId} timed out`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
    if (stepResult.record.error_code === "ENOENT") throw new CollectorError("command_not_found", `${rowId}/${checkId} command was not found`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
    if (stepResult.code !== 0 || stepResult.signal) throw new CollectorError("command_failed", `${rowId}/${checkId} command failed`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
    const text = stepResult.stdout.trim()
    if (!allowEmpty && !text) throw new CollectorError("malformed_output", `${rowId}/${checkId} command returned empty stdout`, { rowId, checkId })
    return { text, stepResult }
  }

  async function runJson(ctx, rowId, checkId, step, command, args, { cwd } = {}) {
    const stepResult = await executeStep(ctx, rowId, checkId, step, command, args, { cwd })
    if (stepResult.timedOut) throw new CollectorError("command_timeout", `${rowId}/${checkId} timed out`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
    if (stepResult.code !== 0 || stepResult.signal) {
      if (stepResult.record.error_code === "ENOENT") throw new CollectorError("command_not_found", `${rowId}/${checkId} command was not found`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
      throw new CollectorError("command_failed", `${rowId}/${checkId} command failed`, { rowId, checkId, evidenceRef: stepResult.evidencePath })
    }
    return { result: resultPayload(stepResult.stdout, rowId, checkId), stepResult }
  }

  function probeArgs(ctx, rowId, checkId) {
    const args = [
      "--source-root", ctx.sourceRoot,
      "--reviewed-commit", ctx.reviewedCommit,
      "--parity-row", rowId,
      "--parity-check", checkId,
      "--topology", ctx.topology,
      "--json",
      "--home-path", "/home",
      "--tmp-path", "/tmp",
      "--nested-path", ctx.nestedPath,
      "--new-directory", ctx.newDirectory,
    ]
    if (rowId === "MP-02" && checkId === "exact_path_entry") {
      args.push("--expected-cwd", ctx.expectedCwd)
    }
    return args
  }

  async function runProbe(ctx, rowId, checkId, normalizer) {
    const probeCwd = rowId === "MP-02" && checkId === "exact_path_entry"
      ? ctx.expectedCwd
      : ctx.sourceRoot
    const probeArguments = [ctx.probePath, ...probeArgs(ctx, rowId, checkId)]
    const { result, stepResult } = await runJson(
      ctx,
      rowId,
      checkId,
      "probe",
      process.execPath,
      probeArguments,
      { cwd: probeCwd },
    )
    if (result.probe_identity_verified !== true
      || result.probe_source_commit !== ctx.reviewedCommit
      || result.probe_file !== PROBE_RELATIVE_PATH
      || result.probe_file_git_blob !== ctx.probeGitBlob) {
      throw new CollectorError("probe_identity_mismatch", `${rowId}/${checkId} did not report the reviewed repo-owned probe`, { rowId, checkId })
    }
    const normalized = normalizer(result)
    return {
      status: "pass",
      result: normalized,
      command: stepResult.commandText,
      evidence_refs: [stepResult.evidencePath],
    }
  }

  async function collect(options) {
    for (const [key, label] of [["kernelBinary", "kernel binary"], ["kernelReleaseRoot", "kernel release root"], ["kernelReleasePublicKey", "kernel release public key"], ["kernelBuilderPublicKey", "trusted builder public key"]]) {
      if (nonEmptyString(options[key]) && !isAbsolute(options[key])) throw new CollectorError("path_invalid", `${label} must be an absolute path`)
    }
    if (
      options.expectedCwd !== undefined
      && (!nonEmptyString(options.expectedCwd) || !isAbsolute(options.expectedCwd))
    ) {
      throw new CollectorError("path_invalid", "expected provider working directory must be absolute")
    }
    const sourceRoot = resolve(options.sourceRoot ?? (typeof processApi.cwd === "function" ? processApi.cwd() : process.cwd()))
    let expectedCwd = sourceRoot
    if (options.expectedCwd !== undefined) {
      const actualInvocationCwd = typeof processApi.cwd === "function" ? processApi.cwd() : process.cwd()
      let actualCanonicalCwd
      let expectedCanonicalCwd
      try {
        const canonicalPaths = await Promise.all([
          filesystem.realpath(resolve(actualInvocationCwd)),
          filesystem.realpath(resolve(options.expectedCwd)),
        ])
        actualCanonicalCwd = canonicalPaths[0]
        expectedCanonicalCwd = canonicalPaths[1]
      } catch (error) {
        throw new CollectorError("cwd_unavailable", "expected provider working directory is not accessible", { cause: error })
      }
      if (actualCanonicalCwd !== expectedCanonicalCwd) {
        throw new CollectorError("cwd_mismatch", "collector was not invoked from the expected provider working directory")
      }
      expectedCwd = expectedCanonicalCwd
    }
    const normalizedOptions = {
      ...options,
      sourceRoot,
      expectedCwd,
      outputPath: options.outputPath ? resolve(options.outputPath) : "",
      evidenceDir: resolve(options.evidenceDir ?? `${options.outputPath}.evidence`),
      kernelBinary: options.kernelBinary ? resolve(options.kernelBinary) : "",
      kernelReleaseRoot: options.kernelReleaseRoot ? resolve(options.kernelReleaseRoot) : "",
      kernelReleasePublicKey: options.kernelReleasePublicKey ? resolve(options.kernelReleasePublicKey) : "",
      kernelBuilderPublicKey: options.kernelBuilderPublicKey ? resolve(options.kernelBuilderPublicKey) : "",
      timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    }
    validateOptions(normalizedOptions, processApi)
    const ctx = {
      ...normalizedOptions,
      filesystem,
      processApi,
      probePath: resolve(normalizedOptions.sourceRoot, PROBE_RELATIVE_PATH),
      releaseVerifierPath: resolve(normalizedOptions.sourceRoot, RELEASE_VERIFIER_RELATIVE_PATH),
      nestedPath: join("/tmp", `chariox-parity-${processApi.pid ?? "collector"}-nested`),
      newDirectory: join("/tmp", `chariox-parity-${processApi.pid ?? "collector"}-created`),
    }
    await filesystem.mkdir(dirname(ctx.outputPath), { recursive: true })
    await filesystem.mkdir(ctx.evidenceDir, { recursive: true })

    const git = ctx.gitCommand ?? "git"
    const kernel = ctx.kernelBinary
    const sourceCommitStep = await runText(ctx, "MP-10", "source_protocol_identity", "source-commit", git, ["rev-parse", "HEAD"], { cwd: ctx.sourceRoot })
    const actualCommit = sourceCommitStep.text
    if (actualCommit !== ctx.reviewedCommit) throw new CollectorError("source_identity_mismatch", `expected ${ctx.reviewedCommit}, observed ${actualCommit}`)
    const cleanStep = await runText(ctx, "MP-10", "source_protocol_identity", "source-clean", git, ["diff", "--quiet", "--exit-code"], { allowEmpty: true, cwd: ctx.sourceRoot })
    if (cleanStep.stepResult.code !== 0) throw new CollectorError("source_dirty", "source worktree is not clean")
    const statusStep = await runText(ctx, "MP-10", "source_protocol_identity", "source-status", git, ["status", "--porcelain=1", "--untracked-files=all"], { allowEmpty: true, cwd: ctx.sourceRoot })
    if (statusStep.text) throw new CollectorError("source_dirty", "source worktree has untracked or modified files")
    const filesStep = await runText(ctx, "MP-10", "source_protocol_identity", "source-files", git, ["ls-files", "-s"], { cwd: ctx.sourceRoot })
    const probeIdentity = await verifyRepoFile(ctx, runText, git, PROBE_RELATIVE_PATH, "probe")
    ctx.probeGitBlob = probeIdentity.gitBlob
    const releaseVerifierIdentity = await verifyRepoFile(ctx, runText, git, RELEASE_VERIFIER_RELATIVE_PATH, "release-verifier")
    const sourceTreeStep = await runText(ctx, "MP-10", "source_protocol_identity", "source-tree", git, ["rev-parse", `${ctx.reviewedCommit}^{tree}`], { cwd: ctx.sourceRoot })
    const kernelReleaseStep = await runText(
      ctx,
      "MP-10",
      "source_protocol_identity",
      "kernel-release-verification",
      process.execPath,
      [ctx.releaseVerifierPath, ctx.kernelReleaseRoot, ctx.kernelReleaseDigest, ctx.kernelReleasePublicKey, ...(ctx.topology === "path1" ? ["path1", ctx.kernelBuilderPublicKey] : [])],
      { allowEmpty: true, cwd: ctx.sourceRoot },
    )
    const expectedKernelPath = resolve(ctx.kernelReleaseRoot, "usr/local/bin/chariox-kernel")
    let selectedKernelRealPath
    let expectedKernelRealPath
    try {
      selectedKernelRealPath = await filesystem.realpath(ctx.kernelBinary)
      expectedKernelRealPath = await filesystem.realpath(expectedKernelPath)
    } catch (error) {
      throw new CollectorError("kernel_release_path_mismatch", "selected kernel binary is not readable from the signed release", { cause: error })
    }
    if (selectedKernelRealPath !== expectedKernelRealPath) {
      throw new CollectorError("kernel_release_path_mismatch", "selected kernel binary is not the signed release artifact")
    }
    let releaseBytes
    try {
      releaseBytes = await filesystem.readFile(join(ctx.kernelReleaseRoot, "usr/lib/chariox/release-manifest.json"))
    } catch (error) {
      throw new CollectorError("kernel_release_unreadable", "signed kernel release identity cannot be read", { cause: error })
    }
    const releaseIdentity = parseReleaseManifest(releaseBytes, {
      reviewedCommit: actualCommit,
      sourceTree: sourceTreeStep.text,
    })
    const versionStep = await runText(ctx, "MP-10", "source_protocol_identity", "kernel-version", kernel, ["--version"], { cwd: ctx.sourceRoot })
    if (versionStep.text !== ctx.buildId) throw new CollectorError("build_identity_mismatch", `expected ${ctx.buildId}, observed ${versionStep.text}`)
    const kernelProtocolStep = await runText(ctx, "MP-10", "source_protocol_identity", "kernel-protocol", kernel, ["--print-local-daemon-protocol-version"], { cwd: ctx.sourceRoot })
    const actualKernelProtocol = parseInteger(kernelProtocolStep.text, "kernel protocol")
    if (actualKernelProtocol !== ctx.kernelProtocol) throw new CollectorError("protocol_identity_mismatch", "kernel protocol mismatch")
    const relayStep = await runText(ctx, "MP-10", "source_protocol_identity", "relay-protocol", git, ["grep", "-h", "-m1", "RELAY_PEER_PROTOCOL_VERSION", "--", ctx.relayProtocolFile ?? "apps/kernel/src/transport/relay_peer.rs"], { cwd: ctx.sourceRoot })
    const actualRelayProtocol = parseRelayVersion(relayStep.text)
    if (actualRelayProtocol !== ctx.relayProtocol) throw new CollectorError("protocol_identity_mismatch", "relay protocol mismatch")

    const providerCommandName = basename(ctx.providerCommand)
    if (providerCommandName !== ctx.provider) throw new CollectorError("provider_identity_mismatch", `provider command basename must be ${ctx.provider}`)
    const providerVersionStep = await runText(ctx, "MP-08", "official_provider_identity", "provider-version", ctx.providerCommand, ["--version"], { cwd: ctx.sourceRoot })
    const providerIdentity = await runProbe(ctx, "MP-08", "official_provider_identity", (result) => normalizeProviderIdentity(result, ctx.provider, providerCommandName))
    const freshWorker = await runProbe(ctx, "MP-10", "fresh_worker", normalizeFreshWorker)
    const captureBoundary = await runProbe(ctx, "MP-10", "capture_boundary", (result) => normalizeBoundary(result, ctx.boundary))

    const rows = {
      "MP-08": {
        checks: {
          official_provider_identity: {
            ...providerIdentity,
            result: { ...providerIdentity.result, provider_version_observed: nonEmptyString(providerVersionStep.text) },
            evidence_refs: [...providerIdentity.evidence_refs, providerVersionStep.stepResult.evidencePath],
          },
        },
      },
      "MP-10": {
        checks: {
          source_protocol_identity: {
            status: "pass",
            result: {
              observed: true,
              source_sha_verified: true,
              source_clean: true,
              kernel_protocol: actualKernelProtocol,
              relay_protocol: actualRelayProtocol,
              build_identity_verified: true,
              kernel_release_verified: true,
              kernel_release_digest: ctx.kernelReleaseDigest,
              kernel_artifact_digest: releaseIdentity.kernelDigest,
              kernel_source_commit: releaseIdentity.sourceCommit,
              probe_identity_verified: true,
            },
            command: commandText(git, ["rev-parse", "HEAD"]),
            evidence_refs: [sourceCommitStep.stepResult.evidencePath, cleanStep.stepResult.evidencePath, statusStep.stepResult.evidencePath, filesStep.stepResult.evidencePath, ...probeIdentity.evidenceRefs, ...releaseVerifierIdentity.evidenceRefs, sourceTreeStep.stepResult.evidencePath, kernelReleaseStep.stepResult.evidencePath, versionStep.stepResult.evidencePath, kernelProtocolStep.stepResult.evidencePath, relayStep.stepResult.evidencePath],
          },
          fresh_worker: freshWorker,
          capture_boundary: captureBoundary,
        },
      },
    }

    for (const definition of ROW_DEFINITIONS) {
      if (definition.id === "MP-07" || definition.id === "MP-09" || definition.id === "MP-10") continue
      const checks = { ...(rows[definition.id]?.checks ?? {}) }
      for (const checkId of definition.checks) {
        if (checkId === "official_provider_identity") {
          continue
        } else if (definition.id === "MP-01" && checkId === "provider_ancestry") {
          checks[checkId] = await runProbe(ctx, definition.id, checkId, normalizeProviderAncestry)
        } else if (definition.id === "MP-01" && checkId === "managed_isolation_environment") {
          checks[checkId] = await runProbe(ctx, definition.id, checkId, normalizeManagedIsolation)
        } else {
          checks[checkId] = await runProbe(ctx, definition.id, checkId, (result) => normalizeGenericResult(result, definition.id, checkId))
        }
      }
      rows[definition.id] = { checks }
    }

    const release = await runProbe(ctx, "MP-07", "signed_release_activation", (result) => normalizeReleaseResult(result, ctx.topology))
    rows["MP-07"] = { checks: { signed_release_activation: release } }
    const shutdownChecks = {}
    for (const checkId of COLLECTOR_CHECKS["MP-09"]) {
      shutdownChecks[checkId] = await runProbe(ctx, "MP-09", checkId, (result) => normalizeShutdownResult(result, ctx.topology, checkId))
    }
    rows["MP-09"] = { checks: shutdownChecks }

    const sourceDigest = `sha256:${sha256(filesStep.text)}`
    const manifest = createSignedManifest({
      schema: MATRIX_SCHEMA,
      manifest_kind: "ordinary-versus-managed-evidence",
      captured_at: nowIso(clock),
      identity: {
        reviewed_commit: actualCommit,
        reviewed_build_id: versionStep.text,
        kernel_build_id: versionStep.text,
        source_digest: sourceDigest,
        kernel_protocol: actualKernelProtocol,
        relay_protocol: actualRelayProtocol,
      },
      topology: ctx.topology,
      worker_fresh: true,
      provider: {
        name: ctx.provider,
        version: providerVersionStep.text,
        executable: providerCommandName,
        official: true,
      },
      collection: {
        boundary: ctx.boundary,
        runtime_probe: true,
        inside_provider_turn: true,
        independent: true,
        fixture: false,
      },
      rows,
    }, ctx.signingKey)
    const validation = validateManifest(manifest, {
      expectedTopology: ctx.topology,
      expectedReviewedCommit: ctx.reviewedCommit,
      expectedBuildId: ctx.buildId,
      signingKey: ctx.signingKey,
      allowFixture: false,
    })
    if (!validation.ok) throw new CollectorError("self_validation_failed", JSON.stringify(validation.failures))
    await filesystem.writeFile(ctx.outputPath, `${JSON.stringify(stable(manifest), null, 2)}\n`, "utf8")
    return manifest
  }

  return { collect }
}

function parseArgs(argv) {
  const values = {}
  const allowed = new Set([
    "topology", "reviewed-commit", "build-id", "kernel-protocol", "relay-protocol", "provider",
    "provider-command", "kernel-binary", "kernel-release-root", "kernel-release-digest",
    "kernel-release-public-key", "kernel-builder-public-key", "boundary", "output", "evidence-dir", "source-root",
    "expected-cwd", "relay-protocol-file", "signing-key-env", "timeout-ms", "help",
  ])
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === "capture") continue
    if (argument === "--help" || argument === "-h") return { help: true }
    if (!argument.startsWith("--")) throw new Error(`unexpected argument: ${argument}`)
    const key = argument.slice(2)
    if (!allowed.has(key)) throw new Error(`unsupported argument: ${argument}`)
    const value = argv[index + 1]
    if (!value || value.startsWith("--")) throw new Error(`missing value for ${argument}`)
    index += 1
    values[key.replaceAll("-", "_")] = value
  }
  return values
}

export function usage() {
  return [
    "node <reviewed-checkout>/apps/cli/scripts/managed-ordinary-parity-collector.mjs capture \\",
    "  --topology ordinary|path1 --reviewed-commit <40-hex> --build-id <kernel-version> \\",
    "  --kernel-protocol <n> --relay-protocol <n> --provider codex|claude|opencode \\",
    "  --provider-command <official-provider> --kernel-binary <chariox-kernel> \\",
    "  --kernel-release-root <verified-rootfs> --kernel-release-digest <sha256:digest> \\",
    "  --kernel-release-public-key <trusted-public-key> --boundary official-provider-turn|remote-command \\",
    "  --kernel-builder-public-key <external-trusted-builder-key> (required for path1) \\",
    "  --source-root <reviewed-checkout> --expected-cwd <provider-working-directory> \\",
    "  --output <manifest.json> \\",
    "  --signing-key-env CHARIOX_PARITY_SIGNING_KEY",
    "  For target-path capture, run this command from that same provider working directory.",
  ].join("\n")
}

export async function runCli(argv = process.argv.slice(2), {
  environment = process.env,
  filesystem = createDefaultFilesystem(),
  runCommand = defaultRunCommand,
  clock = () => new Date(),
  processApi = process,
} = {}) {
  let values
  try {
    values = parseArgs(argv)
  } catch (error) {
    console.error(redactText(error.message))
    console.error(usage())
    return 2
  }
  if (values.help) {
    console.log(usage())
    return 0
  }
  const required = ["topology", "reviewed_commit", "build_id", "kernel_protocol", "relay_protocol", "provider", "provider_command", "kernel_binary", "kernel_release_root", "kernel_release_digest", "kernel_release_public_key", "boundary", "output"]
  const missing = required.filter((key) => !nonEmptyString(values[key]))
  if (missing.length) {
    console.error(`missing required arguments: ${missing.join(", ")}`)
    console.error(usage())
    return 2
  }
  const signingKeyEnv = values.signing_key_env ?? "CHARIOX_PARITY_SIGNING_KEY"
  const signingKey = environment[signingKeyEnv]
  let kernelProtocol
  let relayProtocol
  try {
    kernelProtocol = Number(values.kernel_protocol)
    relayProtocol = Number(values.relay_protocol)
    if (!Number.isInteger(kernelProtocol) || !Number.isInteger(relayProtocol)) throw new Error("protocols must be integers")
    const collector = createParityCollector({ filesystem, runCommand, clock, processApi })
    const manifest = await collector.collect({
      topology: values.topology,
      reviewedCommit: values.reviewed_commit,
      buildId: values.build_id,
      kernelProtocol,
      relayProtocol,
      provider: values.provider,
      providerCommand: values.provider_command,
      kernelBinary: values.kernel_binary,
      kernelReleaseRoot: values.kernel_release_root,
      kernelReleaseDigest: values.kernel_release_digest,
      kernelReleasePublicKey: values.kernel_release_public_key,
      kernelBuilderPublicKey: values.kernel_builder_public_key,
      boundary: values.boundary,
      outputPath: values.output,
      evidenceDir: values.evidence_dir,
      sourceRoot: values.source_root,
      expectedCwd: values.expected_cwd,
      relayProtocolFile: values.relay_protocol_file,
      signingKey,
      timeoutMs: values.timeout_ms ? Number(values.timeout_ms) : DEFAULT_TIMEOUT_MS,
    })
    process.stdout.write(`${JSON.stringify(stable(manifest), null, 2)}\n`)
    return 0
  } catch (error) {
    console.error(`parity collection failed: ${redactText(error.message)}`)
    return 1
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exitCode = await runCli()
}

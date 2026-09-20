#!/usr/bin/env node

import { execFile } from "node:child_process"
import { createHash, createHmac, timingSafeEqual } from "node:crypto"
import { readFile, writeFile } from "node:fs/promises"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)

export const MATRIX_SCHEMA = "chariox.managed-ordinary-parity-matrix/v2"
export const REPORT_SCHEMA = "chariox.managed-ordinary-parity-report/v2"
export const ALLOWED_TOPOLOGIES = Object.freeze(["ordinary", "path1"])
export const ALLOWED_CAPTURE_BOUNDARIES = Object.freeze([
  "official-provider-turn",
  "remote-command",
])

// These IDs match the locked MP-01..MP-10 ledger in
// docs/BROWSER_COMPUTER_USE_END_TO_END_PLAN.md. MP-11 is the separate source
// inventory. Do not renumber runtime checks independently from that ledger.
export const ROW_DEFINITIONS = Object.freeze([
  Object.freeze({
    id: "MP-01",
    title: "ordinary provider launch without managed isolation",
    checks: Object.freeze([
      "provider_ancestry",
      "managed_isolation_environment",
      "mount_visibility",
      "privilege_state",
      "network_reachability",
      "package_tool_installation",
    ]),
  }),
  Object.freeze({
    id: "MP-02",
    title: "directory discovery and exact-path entry",
    checks: Object.freeze([
      "directory_discovery",
      "exact_path_entry",
      "directory_creation",
      "home_access",
      "tmp_access",
    ]),
  }),
  Object.freeze({
    id: "MP-03",
    title: "exact managed control-state protection",
    checks: Object.freeze([
      "control_file_protection",
      "filesystem_permissions",
    ]),
  }),
  Object.freeze({
    id: "MP-04",
    title: "ordinary user home and mutable kernel state",
    checks: Object.freeze(["provider_environment"]),
  }),
  Object.freeze({
    id: "MP-05",
    title: "workspace and repository materialization",
    checks: Object.freeze([
      "empty_workspace",
      "copied_repository",
      "repository_basename",
      "basename_collision",
      "worktree_placement",
    ]),
  }),
  Object.freeze({
    id: "MP-06",
    title: "server-authoritative repository root",
    checks: Object.freeze([
      "repository_root_default",
      "repository_root_custom",
      "repository_root_inheritance",
      "repository_root_override_rejected",
    ]),
  }),
  Object.freeze({
    id: "MP-07",
    title: "signed managed release activation",
    exemption: Object.freeze({
      kind: "managed-signed-release-activation",
      reason: "managed deployment may use signed atomic release activation",
    }),
    checks: Object.freeze(["signed_release_activation"]),
  }),
  Object.freeze({
    id: "MP-08",
    title: "ordinary runtime, protocol, provider, client, and recovery parity",
    checks: Object.freeze([
      "official_provider_identity",
      "session_agent_launch",
      "terminal_file_git",
      "attachments_permissions_capabilities",
      "project_setup",
      "reconnect_orphan_recovery",
      "restart_recovery",
      "reconnect_history_result_identity",
      "queued_prompts",
      "active_turn_state",
      "resource_limits",
      "structured_errors",
      "protocol_behavior",
      "cleanup",
    ]),
  }),
  Object.freeze({
    id: "MP-09",
    title: "mandatory managed automatic shutdown",
    exemption: Object.freeze({
      kind: "managed-automatic-shutdown",
      reason: "managed lifecycle may stop workers under the locked shutdown policy",
    }),
    checks: Object.freeze([
      "shutdown_agents_done",
      "shutdown_idle_15m",
      "shutdown_idle_30m",
      "shutdown_minimum_3h",
      "shutdown_disabled",
      "shutdown_keep_running",
      "shutdown_restart_reconciliation",
      "shutdown_manual",
      "shutdown_custom",
      "shutdown_explicit_lifecycle_reconciliation",
      "shutdown_deployment_reconciliation",
    ]),
  }),
  Object.freeze({
    id: "MP-10",
    title: "fresh-machine comparison identity and evidence",
    checks: Object.freeze([
      "source_protocol_identity",
      "fresh_worker",
      "capture_boundary",
    ]),
  }),
])

export const REQUIRED_ROW_IDS = Object.freeze(ROW_DEFINITIONS.map(({ id }) => id))
export const REQUIRED_CHECK_IDS = Object.freeze(
  Object.fromEntries(ROW_DEFINITIONS.map(({ id, checks }) => [id, [...checks]])),
)

const TOP_LEVEL_KEYS = Object.freeze([
  "schema",
  "manifest_kind",
  "captured_at",
  "identity",
  "topology",
  "worker_fresh",
  "provider",
  "collection",
  "rows",
  "signature",
])
const IDENTITY_KEYS = Object.freeze([
  "reviewed_commit",
  "reviewed_build_id",
  "kernel_build_id",
  "source_digest",
  "kernel_protocol",
  "relay_protocol",
])
const PROVIDER_KEYS = Object.freeze(["name", "version", "executable", "official"])
const COLLECTION_KEYS = Object.freeze([
  "boundary",
  "runtime_probe",
  "inside_provider_turn",
  "independent",
  "fixture",
])
const ROW_KEYS = Object.freeze(["checks"])
const CHECK_KEYS = Object.freeze(["status", "result", "command", "evidence_refs"])
const SIGNATURE_KEYS = Object.freeze(["algorithm", "key_id", "value"])
const RELEASE_RESULT_KEYS = Object.freeze([
  "exemption",
  "observed",
  "signed_release",
  "signature_verified",
  "atomic_activation",
  "rollback_verified",
  "release_digest",
])
const GENERIC_RESULT_REQUIREMENTS = Object.freeze({
  "MP-01/mount_visibility": Object.freeze(["mounts_match_ordinary", "mount_probe_complete"]),
  "MP-01/privilege_state": Object.freeze(["capabilities_match_ordinary", "umask_matches_ordinary"]),
  "MP-01/network_reachability": Object.freeze(["network_matches_ordinary", "address_families_recorded"]),
  "MP-01/package_tool_installation": Object.freeze(["tool_probe_succeeded", "install_probe_succeeded"]),
  "MP-02/directory_discovery": Object.freeze(["exact_path_accessible"]),
  "MP-02/exact_path_entry": Object.freeze(["exact_path_accessible", "cwd_matches_requested"]),
  "MP-02/directory_creation": Object.freeze(["created_and_accessible"]),
  "MP-02/home_access": Object.freeze(["accessible"]),
  "MP-02/tmp_access": Object.freeze(["accessible"]),
  "MP-03/control_file_protection": Object.freeze(["control_file_denied", "sibling_accessible"]),
  "MP-03/filesystem_permissions": Object.freeze(["permissions_match_ordinary"]),
  "MP-04/provider_environment": Object.freeze(["home_matches_ordinary", "chariox_home_matches_ordinary", "cwd_matches_requested", "ordinary_user"]),
  "MP-05/empty_workspace": Object.freeze(["workspace_created", "control_state_separate"]),
  "MP-05/copied_repository": Object.freeze(["repository_accessible"]),
  "MP-05/repository_basename": Object.freeze(["source_basename_preserved"]),
  "MP-05/basename_collision": Object.freeze(["collision_rejected"]),
  "MP-05/worktree_placement": Object.freeze(["worktree_user_path", "control_root_not_workspace"]),
  "MP-06/repository_root_default": Object.freeze(["default_root_correct"]),
  "MP-06/repository_root_custom": Object.freeze(["custom_root_persisted"]),
  "MP-06/repository_root_inheritance": Object.freeze(["child_inherits_root"]),
  "MP-06/repository_root_override_rejected": Object.freeze(["client_override_rejected"]),
  "MP-08/session_agent_launch": Object.freeze(["session_created", "agent_created", "official_command"]),
  "MP-08/terminal_file_git": Object.freeze(["terminal_ok", "file_ok", "git_ok"]),
  "MP-08/attachments_permissions_capabilities": Object.freeze(["attachments_ok", "permissions_ok", "capabilities_ok"]),
  "MP-08/project_setup": Object.freeze(["project_setup_ok"]),
  "MP-08/reconnect_orphan_recovery": Object.freeze(["reconnect_ok", "orphan_recovered"]),
  "MP-08/restart_recovery": Object.freeze(["restart_recovered"]),
  "MP-08/reconnect_history_result_identity": Object.freeze(["history_preserved", "result_identity_preserved"]),
  "MP-08/queued_prompts": Object.freeze(["queued_prompt_preserved", "queued_prompt_advanced"]),
  "MP-08/active_turn_state": Object.freeze(["active_turn_state_preserved"]),
  "MP-08/resource_limits": Object.freeze(["limits_observed"]),
  "MP-08/structured_errors": Object.freeze(["structured_errors"]),
  "MP-08/protocol_behavior": Object.freeze(["protocol_behavior_ok"]),
  "MP-08/cleanup": Object.freeze(["owned_processes_gone", "owned_artifacts_removed", "foreign_processes_untouched", "cleanup_complete"]),
})
const SOURCE_IDENTITY_RESULT_KEYS = Object.freeze([
  "observed",
  "source_sha_verified",
  "source_clean",
  "kernel_protocol",
  "relay_protocol",
  "build_identity_verified",
  "kernel_release_verified",
  "kernel_release_digest",
  "kernel_artifact_digest",
  "kernel_source_commit",
  "probe_identity_verified",
])
const FRESH_WORKER_RESULT_KEYS = Object.freeze([
  "observed",
  "fresh_worker",
  "worker_identity_observed",
])
const PROVIDER_IDENTITY_RESULT_KEYS = Object.freeze([
  "observed",
  "official",
  "provider_name",
  "executable_matches",
  "executable_basename",
  "provider_version_observed",
])
const CAPTURE_BOUNDARY_RESULT_KEYS = Object.freeze([
  "observed",
  "boundary_verified",
  "inside_provider_turn",
  "independent",
])
const PROVIDER_ANCESTRY_RESULT_KEYS = Object.freeze([
  "observed",
  "provider_observed",
  "bwrap_ancestor",
  "fresh_worker",
])
const MANAGED_ISOLATION_RESULT_KEYS = Object.freeze([
  "observed",
  "managed_marker_absent",
  "bwrap_environment_absent",
])
const SHUTDOWN_RESULT_KEYS = Object.freeze([
  "exemption",
  "trigger",
  "expected_outcome",
  "observed_outcome",
  "managed_policy",
  "observation_complete",
  "worker_stopped",
  "cleanup_confirmed",
  "measured_from_last_agent_finished",
  "configured_delay_seconds",
  "observed_delay_seconds",
])
export const SHUTDOWN_EXPECTATIONS = Object.freeze({
  agents_done: Object.freeze({ outcome: "idle-deadline-scheduled", workerStopped: false, cleanupConfirmed: false, measuredFromLastAgentFinished: true, delay: "positive", observesDelay: false }),
  idle_15m: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: true, delay: 900, observesDelay: true }),
  idle_30m: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: true, delay: 1_800, observesDelay: true }),
  minimum_3h: Object.freeze({ outcome: "worker-remained-running", workerStopped: false, cleanupConfirmed: false, measuredFromLastAgentFinished: false, delay: 10_800, observesDelay: true }),
  disabled: Object.freeze({ outcome: "worker-remained-running", workerStopped: false, cleanupConfirmed: false, measuredFromLastAgentFinished: false, delay: null, observesDelay: false }),
  keep_running: Object.freeze({ outcome: "worker-remained-running", workerStopped: false, cleanupConfirmed: false, measuredFromLastAgentFinished: false, delay: null, observesDelay: false }),
  restart_reconciliation: Object.freeze({ outcome: "shutdown-policy-restored", workerStopped: false, cleanupConfirmed: false, measuredFromLastAgentFinished: false, delay: null, observesDelay: false }),
  manual: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: false, delay: 0, observesDelay: true }),
  custom: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: true, delay: "positive", observesDelay: true }),
  explicit_lifecycle_reconciliation: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: false, delay: 0, observesDelay: true }),
  deployment_reconciliation: Object.freeze({ outcome: "worker-stopped", workerStopped: true, cleanupConfirmed: true, measuredFromLastAgentFinished: false, delay: 0, observesDelay: true }),
})
const PARITY_ROW_IDS = new Set(
  ROW_DEFINITIONS.filter((definition) => !definition.exemption).map(({ id }) => id),
)
const RELEASE_ROW_ID = "MP-07"
const SHUTDOWN_ROW_ID = "MP-09"
const REVIEWED_COMMIT = /^[0-9a-f]{40}$/i
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/i
const MIN_SIGNING_KEY_BYTES = 16

const NODE_FILESYSTEM = Object.freeze({ readFile, writeFile })

function isPlainObject(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

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

function asKey(value) {
  if (Buffer.isBuffer(value)) return value
  if (typeof value === "string") return Buffer.from(value)
  return null
}

function keyId(signingKey) {
  return "sha256:" + createHash("sha256").update(signingKey).digest("hex")
}

function payloadWithoutSignature(manifest) {
  const { signature: _signature, ...payload } = manifest
  return payload
}

function signPayload(manifest, signingKey) {
  return createHmac("sha256", signingKey)
    .update(canonicalJson(payloadWithoutSignature(manifest)))
    .digest("base64url")
}

export function createSignedManifest(manifest, signingKey) {
  const key = asKey(signingKey)
  if (!key || key.length < MIN_SIGNING_KEY_BYTES) {
    throw new Error("signing key must contain at least 16 bytes")
  }
  const unsigned = { ...manifest }
  delete unsigned.signature
  return {
    ...unsigned,
    signature: {
      algorithm: "hmac-sha256",
      key_id: keyId(key),
      value: signPayload(unsigned, key),
    },
  }
}

function verifySignature(manifest, signingKey) {
  const key = asKey(signingKey)
  if (!key || key.length < MIN_SIGNING_KEY_BYTES) return "signing_key_missing"
  const signature = manifest?.signature
  if (!isPlainObject(signature)) return "signature_missing"
  if (Object.keys(signature).sort().join("\u0000") !== [...SIGNATURE_KEYS].sort().join("\u0000")) {
    return "signature_shape_invalid"
  }
  if (signature.algorithm !== "hmac-sha256" || signature.key_id !== keyId(key)) {
    return "signature_metadata_invalid"
  }
  if (typeof signature.value !== "string") return "signature_value_invalid"
  const expected = Buffer.from(signPayload(manifest, key))
  const actual = Buffer.from(signature.value)
  if (expected.length !== actual.length || !timingSafeEqual(expected, actual)) {
    return "signature_invalid"
  }
  return null
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function scalarProtocol(value) {
  return nonEmptyString(value) || (typeof value === "number" && Number.isFinite(value))
}

function validBuildId(value) {
  return typeof value === "string" && value.length >= 8 && value.length <= 256
}

function requireComparisonIdentity({ expectedReviewedCommit, expectedBuildId } = {}) {
  if (!REVIEWED_COMMIT.test(expectedReviewedCommit ?? "")) {
    throw new Error("expected reviewed commit must be a 40-hex SHA")
  }
  if (!validBuildId(expectedBuildId)) {
    throw new Error("expected reviewed build id must contain 8 to 256 characters")
  }
}

function sameKeys(actual, expected) {
  if (!isPlainObject(actual)) return false
  const left = Object.keys(actual).sort()
  const right = [...expected].sort()
  return left.length === right.length && left.every((key, index) => key === right[index])
}

function addFailure(failures, code, topology = null, rowId = null, checkId = null, detail = null) {
  failures.push({ code, topology, rowId, checkId, ...(detail ? { detail } : {}) })
}

function validateIdentity(identity, failures, topology, expected = {}) {
  if (!isPlainObject(identity)) {
    addFailure(failures, "identity_missing", topology)
    return
  }
  if (!sameKeys(identity, IDENTITY_KEYS)) {
    addFailure(failures, "identity_shape_invalid", topology)
  }
  if (!REVIEWED_COMMIT.test(identity.reviewed_commit ?? "")) {
    addFailure(failures, "reviewed_commit_invalid", topology)
  }
  for (const key of ["reviewed_build_id", "kernel_build_id", "source_digest"]) {
    if (!nonEmptyString(identity[key])) addFailure(failures, `${key}_invalid`, topology)
  }
  for (const key of ["kernel_protocol", "relay_protocol"]) {
    if (!scalarProtocol(identity[key])) addFailure(failures, `${key}_invalid`, topology)
  }
  for (const [key, value] of Object.entries(expected)) {
    if (value !== undefined && identity[key] !== value) {
      addFailure(failures, "expected_identity_mismatch", topology, null, null, key)
    }
  }
}

function validateProvider(provider, failures, topology) {
  if (!isPlainObject(provider)) {
    addFailure(failures, "provider_missing", topology)
    return
  }
  if (!sameKeys(provider, PROVIDER_KEYS)) addFailure(failures, "provider_shape_invalid", topology)
  for (const key of ["name", "version", "executable"]) {
    if (!nonEmptyString(provider[key])) addFailure(failures, `provider_${key}_invalid`, topology)
  }
  if (provider.official !== true) addFailure(failures, "provider_not_official", topology)
}

function validateCollection(collection, failures, topology, allowFixture) {
  if (!isPlainObject(collection)) {
    addFailure(failures, "collection_missing", topology)
    return
  }
  if (!sameKeys(collection, COLLECTION_KEYS)) addFailure(failures, "collection_shape_invalid", topology)
  if (!ALLOWED_CAPTURE_BOUNDARIES.includes(collection.boundary)) {
    addFailure(failures, "capture_boundary_invalid", topology)
  }
  for (const key of ["runtime_probe", "inside_provider_turn", "independent", "fixture"]) {
    if (typeof collection[key] !== "boolean") addFailure(failures, `${key}_invalid`, topology)
  }
  if (collection.runtime_probe !== true) addFailure(failures, "runtime_probe_required", topology)
  if (collection.inside_provider_turn !== true) addFailure(failures, "provider_turn_required", topology)
  if (collection.independent !== true) addFailure(failures, "independent_capture_required", topology)
  if (collection.fixture === true && !allowFixture) addFailure(failures, "fixture_not_allowed", topology)
}

function checkResultIsObject(result) {
  return isPlainObject(result)
}

function validateReleaseResult(result, topology, rowId, checkId, failures) {
  if (!checkResultIsObject(result)) {
    addFailure(failures, "release_result_invalid", topology, rowId, checkId)
    return
  }
  if (!sameKeys(result, RELEASE_RESULT_KEYS)) {
    addFailure(failures, "release_result_shape_invalid", topology, rowId, checkId)
  }
  if (!nonEmptyString(result.exemption) || !nonEmptyString(result.observed)) {
    addFailure(failures, "release_evidence_missing", topology, rowId, checkId)
  }
  for (const key of ["signed_release", "signature_verified", "atomic_activation", "rollback_verified"]) {
    if (typeof result[key] !== "boolean") {
      addFailure(failures, `release_${key}_invalid`, topology, rowId, checkId)
    }
  }
  if (!nonEmptyString(result.release_digest)) {
    addFailure(failures, "release_digest_missing", topology, rowId, checkId)
  }
  if (topology === "ordinary") {
    if (result.exemption !== "ordinary-not-applicable" || result.signed_release !== false
      || result.signature_verified !== false || result.atomic_activation !== false
      || result.rollback_verified !== false) {
      addFailure(failures, "ordinary_release_exemption_invalid", topology, rowId, checkId)
    }
  } else if (result.exemption !== "managed-signed-release" || result.signed_release !== true
    || result.signature_verified !== true || result.atomic_activation !== true
    || result.rollback_verified !== true) {
    addFailure(failures, "managed_release_evidence_invalid", topology, rowId, checkId)
  }
}

function validateShutdownResult(result, topology, rowId, checkId, failures) {
  if (!checkResultIsObject(result)) {
    addFailure(failures, "shutdown_result_invalid", topology, rowId, checkId)
    return
  }
  if (!sameKeys(result, SHUTDOWN_RESULT_KEYS)) {
    addFailure(failures, "shutdown_result_shape_invalid", topology, rowId, checkId)
  }
  const trigger = checkId.slice("shutdown_".length)
  if (result.trigger !== trigger) addFailure(failures, "shutdown_trigger_mismatch", topology, rowId, checkId)
  const expectation = SHUTDOWN_EXPECTATIONS[trigger]
  if (!expectation) addFailure(failures, "shutdown_trigger_unsupported", topology, rowId, checkId)
  if (!nonEmptyString(result.exemption) || !nonEmptyString(result.expected_outcome)
    || !nonEmptyString(result.observed_outcome)) {
    addFailure(failures, "shutdown_evidence_missing", topology, rowId, checkId)
  }
  for (const key of ["managed_policy", "observation_complete", "worker_stopped", "cleanup_confirmed", "measured_from_last_agent_finished"]) {
    if (typeof result[key] !== "boolean") {
      addFailure(failures, `shutdown_${key}_invalid`, topology, rowId, checkId)
    }
  }
  if (result.observation_complete !== true) {
    addFailure(failures, "shutdown_evidence_missing", topology, rowId, checkId)
  }
  if (topology === "ordinary") {
    if (result.exemption !== "ordinary-no-managed-shutdown" || result.managed_policy !== false
      || result.expected_outcome !== "ordinary-remained-running"
      || result.observed_outcome !== "ordinary-remained-running"
      || result.worker_stopped !== false || result.cleanup_confirmed !== false
      || result.measured_from_last_agent_finished !== false
      || result.configured_delay_seconds !== null || result.observed_delay_seconds !== null) {
      addFailure(failures, "ordinary_shutdown_exemption_invalid", topology, rowId, checkId)
    }
    return
  }
  if (!expectation || result.exemption !== "managed-auto-shutdown" || result.managed_policy !== true
    || result.expected_outcome !== expectation.outcome || result.observed_outcome !== expectation.outcome
    || result.worker_stopped !== expectation.workerStopped
    || result.cleanup_confirmed !== expectation.cleanupConfirmed
    || result.measured_from_last_agent_finished !== expectation.measuredFromLastAgentFinished) {
    addFailure(failures, "managed_shutdown_evidence_invalid", topology, rowId, checkId)
    return
  }
  const configured = result.configured_delay_seconds
  const observed = result.observed_delay_seconds
  if (expectation.delay === null) {
    if (configured !== null || observed !== null) {
      addFailure(failures, "shutdown_delay_invalid", topology, rowId, checkId)
    }
  } else {
    const delayValid = Number.isInteger(configured) && configured >= 0
      && (expectation.delay === "positive" ? configured > 0 : configured === expectation.delay)
    if (!delayValid) addFailure(failures, "shutdown_delay_invalid", topology, rowId, checkId)
    if (expectation.observesDelay) {
      if (!Number.isFinite(observed) || observed < configured || observed > configured + 120) {
        addFailure(failures, "shutdown_observed_delay_invalid", topology, rowId, checkId)
      }
    } else if (observed !== null) {
      addFailure(failures, "shutdown_observed_delay_invalid", topology, rowId, checkId)
    }
  }
}

function validateProviderSafety(rows, topology, failures) {
  const ancestry = rows?.["MP-01"]?.checks?.provider_ancestry?.result
  if (!checkResultIsObject(ancestry) || ancestry.bwrap_ancestor !== false) {
    addFailure(failures, "bwrap_ancestor_present", topology, "MP-01", "provider_ancestry")
  }
  const isolation = rows?.["MP-01"]?.checks?.managed_isolation_environment?.result
  if (!checkResultIsObject(isolation) || isolation.managed_marker_absent !== true) {
    addFailure(failures, "managed_isolation_marker_present", topology, "MP-01", "managed_isolation_environment")
  }
  if (!checkResultIsObject(isolation) || isolation.bwrap_environment_absent !== true) {
    addFailure(failures, "bwrap_environment_marker_present", topology, "MP-01", "managed_isolation_environment")
  }
}

function requireExactTrueResult(result, keys, topology, rowId, checkId, failures) {
  const expectedKeys = ["observed", ...keys]
  if (!checkResultIsObject(result) || !sameKeys(result, expectedKeys)) {
    addFailure(failures, "check_result_shape_invalid", topology, rowId, checkId)
    return
  }
  for (const key of expectedKeys) {
    if (result[key] !== true) {
      addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
    }
  }
}

function validateCheckResult(result, manifest, topology, rowId, checkId, failures) {
  if (rowId === RELEASE_ROW_ID) {
    validateReleaseResult(result, topology, rowId, checkId, failures)
    return
  }
  if (rowId === SHUTDOWN_ROW_ID) {
    validateShutdownResult(result, topology, rowId, checkId, failures)
    return
  }
  if (rowId === "MP-10" && checkId === "source_protocol_identity") {
    if (!checkResultIsObject(result) || !sameKeys(result, SOURCE_IDENTITY_RESULT_KEYS)) {
      addFailure(failures, "check_result_shape_invalid", topology, rowId, checkId)
      return
    }
    for (const key of [
      "observed",
      "source_sha_verified",
      "source_clean",
      "build_identity_verified",
      "kernel_release_verified",
      "probe_identity_verified",
    ]) {
      if (result[key] !== true) addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
    }
    for (const key of ["kernel_release_digest", "kernel_artifact_digest"]) {
      if (!SHA256_DIGEST.test(result[key] ?? "")) {
        addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
      }
    }
    if (result.kernel_source_commit !== manifest?.identity?.reviewed_commit) {
      addFailure(failures, "check_result_identity_mismatch", topology, rowId, checkId, "kernel_source_commit")
    }
    if (result.kernel_protocol !== manifest?.identity?.kernel_protocol) {
      addFailure(failures, "check_result_identity_mismatch", topology, rowId, checkId, "kernel_protocol")
    }
    if (result.relay_protocol !== manifest?.identity?.relay_protocol) {
      addFailure(failures, "check_result_identity_mismatch", topology, rowId, checkId, "relay_protocol")
    }
    return
  }
  if (rowId === "MP-10" && checkId === "fresh_worker") {
    requireExactTrueResult(result, FRESH_WORKER_RESULT_KEYS.slice(1), topology, rowId, checkId, failures)
    return
  }
  if (rowId === "MP-08" && checkId === "official_provider_identity") {
    if (!checkResultIsObject(result) || !sameKeys(result, PROVIDER_IDENTITY_RESULT_KEYS)) {
      addFailure(failures, "check_result_shape_invalid", topology, rowId, checkId)
      return
    }
    for (const key of ["observed", "official", "executable_matches", "provider_version_observed"]) {
      if (result[key] !== true) addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
    }
    if (result.provider_name !== manifest?.provider?.name) {
      addFailure(failures, "check_result_identity_mismatch", topology, rowId, checkId, "provider_name")
    }
    if (result.executable_basename !== manifest?.provider?.executable) {
      addFailure(failures, "check_result_identity_mismatch", topology, rowId, checkId, "executable_basename")
    }
    return
  }
  if (rowId === "MP-10" && checkId === "capture_boundary") {
    requireExactTrueResult(result, CAPTURE_BOUNDARY_RESULT_KEYS.slice(1), topology, rowId, checkId, failures)
    return
  }
  if (rowId === "MP-01" && checkId === "provider_ancestry") {
    if (!checkResultIsObject(result) || !sameKeys(result, PROVIDER_ANCESTRY_RESULT_KEYS)) {
      addFailure(failures, "check_result_shape_invalid", topology, rowId, checkId)
      return
    }
    for (const key of ["observed", "provider_observed", "fresh_worker"]) {
      if (result[key] !== true) addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
    }
    if (result.bwrap_ancestor !== false) {
      addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, "bwrap_ancestor")
    }
    return
  }
  if (rowId === "MP-01" && checkId === "managed_isolation_environment") {
    requireExactTrueResult(result, MANAGED_ISOLATION_RESULT_KEYS.slice(1), topology, rowId, checkId, failures)
    return
  }
  const required = GENERIC_RESULT_REQUIREMENTS[`${rowId}/${checkId}`]
  if (!required) {
    addFailure(failures, "check_result_validator_missing", topology, rowId, checkId)
    return
  }
  const extraKeys = []
  if (rowId === "MP-02" && checkId === "directory_discovery") extraKeys.push("child_enumeration_denied")
  if (rowId === "MP-01" && checkId === "privilege_state") extraKeys.push("no_new_privs")
  const expectedKeys = ["observed", ...required, ...extraKeys]
  if (!checkResultIsObject(result) || !sameKeys(result, expectedKeys)) {
    addFailure(failures, "check_result_shape_invalid", topology, rowId, checkId)
    return
  }
  if (result.observed !== true) {
    addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, "observed")
  }
  for (const key of required) {
    if (result[key] !== true) addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, key)
  }
  if (extraKeys.includes("child_enumeration_denied") && typeof result.child_enumeration_denied !== "boolean") {
    addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, "child_enumeration_denied")
  }
  if (extraKeys.includes("no_new_privs") && result.no_new_privs !== false) {
    addFailure(failures, "check_result_semantics_invalid", topology, rowId, checkId, "no_new_privs")
  }
}

export function validateManifest(manifest, {
  expectedTopology,
  expectedReviewedCommit,
  expectedBuildId,
  signingKey,
  allowFixture = false,
} = {}) {
  const failures = []
  const topology = manifest?.topology ?? expectedTopology ?? null
  if (!isPlainObject(manifest)) {
    addFailure(failures, "manifest_not_object", topology)
    return { ok: false, failures }
  }
  if (!sameKeys(manifest, TOP_LEVEL_KEYS)) addFailure(failures, "manifest_shape_invalid", topology)
  if (manifest.schema !== MATRIX_SCHEMA) addFailure(failures, "schema_mismatch", topology)
  if (manifest.manifest_kind !== "ordinary-versus-managed-evidence") {
    addFailure(failures, "manifest_kind_mismatch", topology)
  }
  if (!ALLOWED_TOPOLOGIES.includes(manifest.topology)) addFailure(failures, "topology_invalid", topology)
  if (expectedTopology && manifest.topology !== expectedTopology) addFailure(failures, "topology_mismatch", topology)
  if (!nonEmptyString(manifest.captured_at)) addFailure(failures, "captured_at_invalid", topology)
  if (manifest.worker_fresh !== true) addFailure(failures, "fresh_worker_required", topology)
  validateIdentity(manifest.identity, failures, topology, {
    reviewed_commit: expectedReviewedCommit,
    reviewed_build_id: expectedBuildId,
  })
  validateProvider(manifest.provider, failures, topology)
  validateCollection(manifest.collection, failures, topology, allowFixture)

  const signatureFailure = verifySignature(manifest, signingKey)
  if (signatureFailure) addFailure(failures, signatureFailure, topology)

  if (!isPlainObject(manifest.rows)) {
    addFailure(failures, "rows_missing", topology)
    return { ok: false, failures }
  }
  const rowIds = Object.keys(manifest.rows)
  for (const rowId of REQUIRED_ROW_IDS) {
    if (!Object.hasOwn(manifest.rows, rowId)) {
      addFailure(failures, "missing_row", topology, rowId)
    }
  }
  for (const rowId of rowIds) {
    if (!REQUIRED_ROW_IDS.includes(rowId)) addFailure(failures, "extra_row", topology, rowId)
  }

  for (const definition of ROW_DEFINITIONS) {
    const row = manifest.rows[definition.id]
    if (!row) continue
    if (!sameKeys(row, ROW_KEYS)) addFailure(failures, "row_shape_invalid", topology, definition.id)
    if (!isPlainObject(row.checks)) {
      addFailure(failures, "checks_missing", topology, definition.id)
      continue
    }
    for (const checkId of definition.checks) {
      const check = row.checks[checkId]
      if (!Object.hasOwn(row.checks, checkId)) {
        addFailure(failures, "missing_check", topology, definition.id, checkId)
        continue
      }
      if (!isPlainObject(check)) {
        addFailure(failures, "check_not_object", topology, definition.id, checkId)
        continue
      }
      if (!sameKeys(check, CHECK_KEYS)) addFailure(failures, "check_shape_invalid", topology, definition.id, checkId)
      if (check.status !== "pass") addFailure(failures, "check_not_pass", topology, definition.id, checkId)
      if (!Object.hasOwn(check, "result") || check.result === null || check.result === undefined) {
        addFailure(failures, "check_result_missing", topology, definition.id, checkId)
      }
      if (!nonEmptyString(check.command)) addFailure(failures, "command_missing", topology, definition.id, checkId)
      if (!Array.isArray(check.evidence_refs) || check.evidence_refs.length === 0
        || check.evidence_refs.some((reference) => !nonEmptyString(reference))) {
        addFailure(failures, "evidence_reference_missing", topology, definition.id, checkId)
      }
      validateCheckResult(check.result, manifest, topology, definition.id, checkId, failures)
    }
    for (const checkId of Object.keys(row.checks)) {
      if (!definition.checks.includes(checkId)) addFailure(failures, "extra_check", topology, definition.id, checkId)
    }
  }
  validateProviderSafety(manifest.rows, topology, failures)
  return { ok: failures.length === 0, failures }
}

function identityFields() {
  return [...IDENTITY_KEYS]
}

function evidenceSummary(check) {
  if (!isPlainObject(check)) return { command: null, evidenceRefs: [] }
  return {
    command: nonEmptyString(check.command) ? check.command : null,
    evidenceRefs: Array.isArray(check.evidence_refs)
      ? check.evidence_refs.filter((reference) => nonEmptyString(reference))
      : [],
  }
}

function failureKey(failure) {
  return [failure.code, failure.topology, failure.rowId, failure.checkId, failure.detail].join("\u0000")
}

function dedupeFailures(failures) {
  const seen = new Set()
  return failures.filter((failure) => {
    const key = failureKey(failure)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

function reportFailure(failure) {
  return {
    code: failure.code,
    ...(failure.topology ? { topology: failure.topology } : {}),
    ...(failure.rowId ? { rowId: failure.rowId } : {}),
    ...(failure.checkId ? { checkId: failure.checkId } : {}),
    ...(failure.detail ? { detail: failure.detail } : {}),
  }
}

function exemptionFor(definition) {
  return definition.exemption ? { ...definition.exemption } : null
}

function identityReport(ordinary, path1) {
  const ordinaryIdentity = isPlainObject(ordinary?.identity) ? ordinary.identity : null
  const path1Identity = isPlainObject(path1?.identity) ? path1.identity : null
  return {
    ordinary: ordinaryIdentity ? Object.fromEntries(identityFields().map((key) => [key, ordinaryIdentity[key] ?? null])) : null,
    path1: path1Identity ? Object.fromEntries(identityFields().map((key) => [key, path1Identity[key] ?? null])) : null,
  }
}

function appendComparisonFailure(failures, code, rowId, checkId, detail = null) {
  addFailure(failures, code, "both", rowId, checkId, detail)
}

function compareIdentity(ordinary, path1, failures) {
  if (!isPlainObject(ordinary?.identity) || !isPlainObject(path1?.identity)) return
  for (const field of identityFields()) {
    if (canonicalJson(ordinary.identity[field]) !== canonicalJson(path1.identity[field])) {
      addFailure(failures, "identity_mismatch", "both", "MP-10", "source_protocol_identity", field)
    }
  }
  for (const field of PROVIDER_KEYS) {
    if (canonicalJson(ordinary?.provider?.[field]) !== canonicalJson(path1?.provider?.[field])) {
      addFailure(failures, "provider_identity_mismatch", "both", "MP-08", "official_provider_identity", field)
    }
  }
}

function compareRows(ordinary, path1, failures) {
  const reports = []
  for (const definition of ROW_DEFINITIONS) {
    const ordinaryRow = ordinary?.rows?.[definition.id]
    const path1Row = path1?.rows?.[definition.id]
    const rowReport = {
      id: definition.id,
      title: definition.title,
      status: "pass",
      exemption: exemptionFor(definition),
      checks: [],
    }
    for (const checkId of definition.checks) {
      const ordinaryCheck = ordinaryRow?.checks?.[checkId]
      const path1Check = path1Row?.checks?.[checkId]
      const checkReport = {
        id: checkId,
        status: "pass",
        comparison: definition.exemption ? "allowed-managed-exemption" : "equal",
        ordinary: evidenceSummary(ordinaryCheck),
        path1: evidenceSummary(path1Check),
        failures: [],
      }
      if (!ordinaryCheck || !path1Check) {
        const missingTopology = !ordinaryCheck && !path1Check ? "both" : (!ordinaryCheck ? "ordinary" : "path1")
        const missing = { code: "missing_check", topology: missingTopology, rowId: definition.id, checkId }
        failures.push(missing)
        checkReport.failures.push(reportFailure(missing))
      } else if (!definition.exemption && canonicalJson(ordinaryCheck.result) !== canonicalJson(path1Check.result)) {
        const difference = { code: "unapproved_difference", topology: "both", rowId: definition.id, checkId }
        failures.push(difference)
        checkReport.failures.push(reportFailure(difference))
      }
      if (checkReport.failures.length > 0) {
        checkReport.status = "fail"
        rowReport.status = "fail"
      }
      rowReport.checks.push(checkReport)
    }
    reports.push(rowReport)
  }
  return reports
}

export function compareManifests(ordinary, path1, {
  expectedReviewedCommit,
  expectedBuildId,
  signingKey,
  allowFixture = false,
} = {}) {
  requireComparisonIdentity({ expectedReviewedCommit, expectedBuildId })
  const ordinaryValidation = validateManifest(ordinary, {
    expectedTopology: "ordinary",
    expectedReviewedCommit,
    expectedBuildId,
    signingKey,
    allowFixture,
  })
  const path1Validation = validateManifest(path1, {
    expectedTopology: "path1",
    expectedReviewedCommit,
    expectedBuildId,
    signingKey,
    allowFixture,
  })
  const failures = [
    ...ordinaryValidation.failures,
    ...path1Validation.failures,
  ]
  // Report both the manifest-local validation failure and the cross-manifest
  // identity mismatch. This keeps a stale commit/protocol diagnosis complete
  // instead of hiding the second mismatch behind the first fail-closed gate.
  compareIdentity(ordinary, path1, failures)
  if (ordinaryValidation.ok && path1Validation.ok) {
    const rowReports = compareRows(ordinary, path1, failures)
    const report = {
      schema: REPORT_SCHEMA,
      status: failures.length === 0 ? "pass" : "fail",
      reviewedIdentity: identityReport(ordinary, path1),
      rows: rowReports,
      failures: dedupeFailures(failures).map(reportFailure),
    }
    for (const row of report.rows) {
      const rowFailure = report.failures.some((failure) => failure.rowId === row.id)
      if (rowFailure) row.status = "fail"
      for (const check of row.checks) {
        if (report.failures.some((failure) => failure.rowId === row.id && failure.checkId === check.id)) {
          check.status = "fail"
          if (!check.failures.length) {
            check.failures = report.failures
              .filter((failure) => failure.rowId === row.id && failure.checkId === check.id)
          }
        }
      }
    }
    return report
  }

  const rowReports = ROW_DEFINITIONS.map((definition) => ({
    id: definition.id,
    title: definition.title,
    status: "fail",
    exemption: exemptionFor(definition),
    checks: definition.checks.map((checkId) => ({
      id: checkId,
      status: "fail",
      comparison: definition.exemption ? "allowed-managed-exemption" : "equal",
      ordinary: evidenceSummary(ordinary?.rows?.[definition.id]?.checks?.[checkId]),
      path1: evidenceSummary(path1?.rows?.[definition.id]?.checks?.[checkId]),
      failures: [],
    })),
  }))
  return {
    schema: REPORT_SCHEMA,
    status: "fail",
    reviewedIdentity: identityReport(ordinary, path1),
    rows: rowReports,
    failures: dedupeFailures(failures).map(reportFailure),
  }
}

export function serializeReport(report) {
  return JSON.stringify(stableValue(report), null, 2) + "\n"
}

export async function defaultRunCommand(command, args = [], options = {}) {
  const result = await execFileAsync(command, args, {
    encoding: "utf8",
    ...options,
  })
  return { code: 0, signal: null, stdout: result.stdout ?? "", stderr: result.stderr ?? "" }
}

export async function runEvidenceCommand(command, args = [], {
  runCommand = defaultRunCommand,
  ...options
} = {}) {
  try {
    const result = await runCommand(command, args, options)
    return {
      ok: true,
      code: result?.code ?? 0,
      signal: result?.signal ?? null,
      stdout: String(result?.stdout ?? ""),
      stderr: String(result?.stderr ?? ""),
    }
  } catch (error) {
    return {
      ok: false,
      code: error?.code ?? null,
      signal: error?.signal ?? null,
      stdout: String(error?.stdout ?? ""),
      stderr: String(error?.stderr ?? error?.message ?? ""),
    }
  }
}

export function createParityMatrixRunner({
  filesystem = NODE_FILESYSTEM,
  runCommand = defaultRunCommand,
} = {}) {
  if (!filesystem || typeof filesystem.readFile !== "function" || typeof filesystem.writeFile !== "function") {
    throw new TypeError("filesystem must provide readFile and writeFile")
  }
  return {
    async loadManifest(filePath) {
      const raw = await filesystem.readFile(filePath, "utf8")
      return JSON.parse(String(raw))
    },
    async compareFiles({ ordinaryPath, path1Path, reportPath, ...options }) {
      requireComparisonIdentity(options)
      const ordinary = await this.loadManifest(ordinaryPath)
      const path1 = await this.loadManifest(path1Path)
      const report = compareManifests(ordinary, path1, options)
      if (reportPath) await filesystem.writeFile(reportPath, serializeReport(report), "utf8")
      return report
    },
    runEvidenceCommand(command, args = [], options = {}) {
      return runEvidenceCommand(command, args, { ...options, runCommand })
    },
  }
}

function parseArgs(argv) {
  const args = [...argv]
  if (args[0] === "compare") args.shift()
  const values = {}
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]
    if (argument === "--help" || argument === "-h") return { help: true }
    if (!argument.startsWith("--")) throw new Error(`unexpected argument: ${argument}`)
    const key = argument.slice(2).replaceAll("-", "_")
    const value = args[index + 1]
    if (!value || value.startsWith("--")) throw new Error(`missing value for --${key.replaceAll("_", "-")}`)
    values[key] = value
    index += 1
  }
  return values
}

export function usage() {
  return [
    "node apps/cli/scripts/managed-ordinary-parity-matrix.mjs compare \\",
    "  --ordinary ordinary.json --path1 path1.json --report parity-report.json \\",
    "  --reviewed-commit <40-hex> --build-id <id> [--signing-key-env CHARIOX_PARITY_SIGNING_KEY]",
  ].join("\n")
}

export async function runCli(argv = process.argv.slice(2), {
  filesystem = NODE_FILESYSTEM,
  environment = process.env,
} = {}) {
  let options
  try {
    options = parseArgs(argv)
  } catch (error) {
    console.error(error.message)
    console.error(usage())
    return 2
  }
  if (options.help) {
    console.log(usage())
    return 0
  }
  const ordinaryPath = options.ordinary
  const path1Path = options.path1
  const reportPath = options.report
  if (!ordinaryPath || !path1Path || !reportPath || !options.reviewed_commit || !options.build_id) {
    console.error("--ordinary, --path1, --report, --reviewed-commit, and --build-id are required")
    console.error(usage())
    return 2
  }
  const signingKeyEnv = options.signing_key_env ?? "CHARIOX_PARITY_SIGNING_KEY"
  const signingKey = environment[signingKeyEnv]
  const runner = createParityMatrixRunner({ filesystem })
  try {
    const report = await runner.compareFiles({
      ordinaryPath,
      path1Path,
      reportPath,
      signingKey,
      expectedReviewedCommit: options.reviewed_commit,
      expectedBuildId: options.build_id,
    })
    const output = serializeReport(report)
    process.stdout.write(output)
    return report.status === "pass" ? 0 : 1
  } catch (error) {
    console.error(`parity matrix failed to run: ${error.message}`)
    return 2
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const exitCode = await runCli()
  process.exitCode = exitCode
}

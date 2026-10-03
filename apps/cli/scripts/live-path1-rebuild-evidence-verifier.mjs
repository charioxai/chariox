#!/usr/bin/env node

// This gate reads explicit, campaign-bound receipts and retained evidence
// references. It never contacts or mutates the provider, Cloud, relay, or host.

import { createHash } from "node:crypto"
import { lstat, readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"

export const EVIDENCE_SCHEMA = "chariox.path1-rebuild-evidence/v1"
export const REPORT_SCHEMA = "chariox.path1-rebuild-evidence-report/v1"

const DIGEST = /^sha256:[0-9a-f]{64}$/
const GIT_OBJECT_ID = /^[0-9a-f]{40}$/
const PLACEHOLDER = /(?:^|\b)(?:unknown|unset|placeholder|n\/a|none|todo|tbd|default)(?:\b|$)|<[^>]+>/i
const MAX_RECEIPT_BYTES = 1024 * 1024
const MAX_EVIDENCE_BYTES = 4 * 1024 * 1024
const RECEIPT_KINDS = Object.freeze(["reviewed", "before", "rebuild", "after", "cleanup"])

export class RebuildEvidenceError extends Error {
  constructor(code, message) {
    super(message)
    this.name = "RebuildEvidenceError"
    this.code = code
  }
}

function fail(code, message) {
  throw new RebuildEvidenceError(code, message)
}

function plainObject(value, label, expectedKeys) {
  if (!value || typeof value !== "object" || Array.isArray(value)
    || ![Object.prototype, null].includes(Object.getPrototypeOf(value))) {
    fail("receipt_shape", `${label} must be an object`)
  }
  if (expectedKeys) {
    const actual = Object.keys(value).sort()
    const expected = [...expectedKeys].sort()
    if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
      fail("receipt_shape", `${label} has missing or unsupported fields`)
    }
  }
  return value
}

function nonEmpty(value, label, minLength = 1) {
  if (typeof value !== "string" || value.trim().length < minLength || PLACEHOLDER.test(value)) {
    fail("receipt_value", `${label} is missing or is a placeholder`)
  }
  return value
}

function timestamp(value, label) {
  if (typeof value !== "string" || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)
    || !Number.isFinite(Date.parse(value)) || new Date(value).toISOString() !== value) {
    fail("receipt_time", `${label} must be a UTC timestamp`)
  }
  return Date.parse(value)
}

function captureReceipt(receipt, kind, expectedAuthorities) {
  plainObject(receipt, `${kind} receipt`, ["schema", "kind", "campaignId", "capturedAt", "observations", ...expectedAuthorities.fields])
  if (receipt.schema !== EVIDENCE_SCHEMA || receipt.kind !== kind) {
    fail("receipt_kind", `${kind} receipt has the wrong schema or kind`)
  }
  const campaignId = nonEmpty(receipt.campaignId, `${kind}.campaignId`, 8)
  const capturedAt = timestamp(receipt.capturedAt, `${kind}.capturedAt`)
  if (!Array.isArray(receipt.observations) || receipt.observations.length > 64) {
    fail("receipt_observations", `${kind}.observations must be a bounded array`)
  }
  const observations = new Map()
  const authorities = new Set()
  for (const [index, value] of receipt.observations.entries()) {
    const item = plainObject(value, `${kind}.observations[${index}]`, [
      "id", "authority", "operation", "capturedAt", "evidenceRef", "evidenceSha256", "exitCode",
    ])
    const id = nonEmpty(item.id, `${kind} observation id`)
    const authority = nonEmpty(item.authority, `${kind} observation authority`)
    const operation = nonEmpty(item.operation, `${kind} observation operation`)
    const evidenceRef = nonEmpty(item.evidenceRef, `${kind} evidence reference`, 8)
    if (!evidenceRef.startsWith("file:/") || !DIGEST.test(item.evidenceSha256)) {
      fail("receipt_evidence_ref", `${kind} must bind to an absolute retained capture file and its SHA-256`)
    }
    const observedAt = timestamp(item.capturedAt, `${kind} observation timestamp`)
    if (!expectedAuthorities.allowed.includes(authority)) {
      fail("receipt_authority", `${kind} contains an unsupported evidence authority`)
    }
    if (item.exitCode !== 0 || observedAt > capturedAt) {
      fail("receipt_observation_failed", `${kind} contains a failed or future-dated observation`)
    }
    if (observations.has(id)) fail("receipt_observations", `${kind} observation ids must be unique`)
    observations.set(id, { id, authority, operation, evidenceRef, evidenceSha256: item.evidenceSha256, capturedAt: observedAt })
    authorities.add(authority)
  }
  for (const authority of expectedAuthorities.required) {
    if (!authorities.has(authority)) fail("receipt_observation_missing", `${kind} lacks ${authority} evidence`)
  }
  return { campaignId, capturedAt, observations }
}

function requireObservation(receiptInfo, id, authority, operationFragment, after = undefined) {
  const value = receiptInfo.observations.get(id)
  if (!value || value.authority !== authority || !value.operation.toLowerCase().includes(operationFragment)) {
    fail("receipt_observation_missing", `required ${authority} observation ${id} is missing`)
  }
  if (after !== undefined && value.capturedAt <= after) {
    fail("receipt_stale", `observation ${id} predates the required transition`)
  }
  return value
}

function requireCapturedOperation(receiptInfo, authority, fragments) {
  const observation = [...receiptInfo.observations.values()].find((item) => item.authority === authority)
  const operation = observation?.operation.toLowerCase() ?? ""
  if (!observation || fragments.some((fragment) => !operation.includes(fragment))) {
    fail("receipt_observation_missing", `required ${authority} capture does not describe ${fragments.join(" and ")}`)
  }
  return observation
}

function allocation(value, label) {
  plainObject(value, label, ["provider", "projectId", "serverId", "serverType", "datacenter", "imageId"])
  if (value.provider !== "hetzner") fail("allocation_provider", `${label} must identify the approved Hetzner allocation`)
  for (const field of ["projectId", "serverId", "serverType", "datacenter", "imageId"]) {
    nonEmpty(value[field], `${label}.${field}`)
  }
  return value
}

function sameAllocation(before, after, label, includeImage = false) {
  for (const field of ["provider", "projectId", "serverId", "serverType", "datacenter", ...(includeImage ? ["imageId"] : [])]) {
    if (before[field] !== after[field]) fail("allocation_mismatch", `${label}.${field} contradicts the captured allocation`)
  }
}

function uniqueStrings(values, label, min = 1) {
  if (!Array.isArray(values) || values.length < min || values.length > 10_000) {
    fail("identity_missing", `${label} must contain captured identities`)
  }
  const checked = values.map((value, index) => nonEmpty(value, `${label}[${index}]`, 3))
  if (new Set(checked).size !== checked.length) fail("identity_duplicate", `${label} contains duplicate identities`)
  return checked
}

function identityPair(value, label, keys) {
  plainObject(value, label, keys)
  for (const key of keys) nonEmpty(value[key], `${label}.${key}`, 3)
  return value
}

function validateRelease(value, label, { instance = false, verified = !instance } = {}) {
  const keys = ["digest", "sourceCommit", "sourceTree", ...(instance ? ["instanceId"] : []),
    ...(verified ? ["signatureVerified", "manifestDigestVerified", "kernelArtifactVerified"] : [])]
  plainObject(value, label, keys)
  if (!DIGEST.test(value.digest)) fail("release_digest", `${label}.digest is invalid`)
  if (!GIT_OBJECT_ID.test(value.sourceCommit) || !GIT_OBJECT_ID.test(value.sourceTree)) {
    fail("release_source", `${label} source identity is invalid`)
  }
  if (instance) nonEmpty(value.instanceId, `${label}.instanceId`, 3)
  if (verified && (value.signatureVerified !== true || value.manifestDigestVerified !== true || value.kernelArtifactVerified !== true)) {
    fail("release_unverified", `${label} lacks signed digest and kernel-artifact verification`)
  }
  return value
}

function validateReviewed(reviewed) {
  const info = captureReceipt(reviewed, "reviewed", {
    fields: ["reviewedCommit", "approvedImage", "release"],
    allowed: ["git-command", "release-verifier", "image-approval"],
    required: ["git-command", "release-verifier", "image-approval"],
  })
  if (!GIT_OBJECT_ID.test(reviewed.reviewedCommit)) fail("reviewed_commit", "reviewed source commit is invalid")
  const approvedImage = plainObject(reviewed.approvedImage, "reviewed.approvedImage", [
    "imageId", "name", "type", "approved", "cleanBase", "approvalRef",
  ])
  for (const field of ["imageId", "name", "type", "approvalRef"]) nonEmpty(approvedImage[field], `reviewed.approvedImage.${field}`)
  if (approvedImage.approved !== true || approvedImage.cleanBase !== true) {
    fail("image_not_approved", "reviewed image is not recorded as an approved clean base image")
  }
  const approval = [...info.observations.values()].find((item) => item.authority === "image-approval")
  if (!approval || approval.evidenceRef !== approvedImage.approvalRef) {
    fail("image_approval_evidence", "approved image does not bind to its captured approval record")
  }
  const release = validateRelease(reviewed.release, "reviewed.release")
  if (release.sourceCommit !== reviewed.reviewedCommit) {
    fail("release_source_mismatch", "signed release source commit differs from the reviewed commit")
  }
  requireObservation(info, [...info.observations.values()].find((item) => item.authority === "git-command")?.id,
    "git-command", "rev-parse")
  requireObservation(info, [...info.observations.values()].find((item) => item.authority === "release-verifier")?.id,
    "release-verifier", "verify-image-release")
  requireCapturedOperation(info, "image-approval", ["clean", "approval"])
  return { info, approvedImage, release }
}

function validateBefore(before) {
  const info = captureReceipt(before, "before", {
    fields: ["allocation", "identity"],
    allowed: ["hetzner-api", "host-command", "cloud-api", "relay-api"],
    required: ["hetzner-api", "host-command", "cloud-api", "relay-api"],
  })
  const resource = allocation(before.allocation, "before.allocation")
  requireCapturedOperation(info, "hetzner-api", ["server"])
  requireCapturedOperation(info, "host-command", ["systemd"])
  requireCapturedOperation(info, "cloud-api", ["enrollment"])
  requireCapturedOperation(info, "relay-api", ["heartbeat"])
  const identity = plainObject(before.identity, "before.identity", [
    "bootId", "machineId", "enrollmentId", "relayRegistrationId", "runtimeMachineId", "release",
    "serviceInstances", "processes", "stateInstances", "cloudRows", "heartbeats",
  ])
  for (const field of ["bootId", "machineId", "enrollmentId", "relayRegistrationId", "runtimeMachineId"]) {
    nonEmpty(identity[field], `before.identity.${field}`, 3)
  }
  validateRelease(identity.release, "before.identity.release", { instance: true })

  if (!Array.isArray(identity.serviceInstances) || identity.serviceInstances.length === 0) {
    fail("old_services_missing", "before receipt must identify old service instances")
  }
  const services = identity.serviceInstances.map((item, index) => identityPair(item, `before service ${index}`, ["unit", "invocationId"]))
  uniqueStrings(services.map(({ unit }) => unit), "before service units")
  uniqueStrings(services.map(({ invocationId }) => invocationId), "before service invocation ids")
  const processes = uniqueStrings(identity.processes, "before process identities")

  if (!Array.isArray(identity.stateInstances) || identity.stateInstances.length === 0) {
    fail("old_state_missing", "before receipt must identify old state roots")
  }
  const states = identity.stateInstances.map((item, index) => identityPair(item, `before state ${index}`, ["path", "instanceId"]))
  for (const requiredPath of ["/var/lib/chariox", "/home/chariox"]) {
    if (!states.some((item) => item.path === requiredPath)) fail("old_state_missing", `before receipt lacks ${requiredPath} identity`)
  }
  uniqueStrings(states.map(({ path }) => path), "before state paths")
  uniqueStrings(states.map(({ instanceId }) => instanceId), "before state instance ids")

  if (!Array.isArray(identity.cloudRows) || identity.cloudRows.length === 0) {
    fail("old_cloud_rows_missing", "before receipt must identify old Cloud control rows")
  }
  const cloudRows = identity.cloudRows.map((item, index) => identityPair(item, `before Cloud row ${index}`, ["rowId", "identity"]))
  uniqueStrings(cloudRows.map(({ rowId }) => rowId), "before Cloud row ids")
  for (const oldId of [identity.enrollmentId, identity.runtimeMachineId]) {
    if (!cloudRows.some((item) => item.identity === oldId)) fail("old_cloud_rows_missing", "before Cloud rows omit the old enrollment or runtime identity")
  }

  if (!Array.isArray(identity.heartbeats) || identity.heartbeats.length === 0) {
    fail("old_heartbeats_missing", "before receipt must identify old relay heartbeats")
  }
  const heartbeats = identity.heartbeats.map((item, index) => identityPair(item, `before heartbeat ${index}`, ["heartbeatId", "identity"]))
  uniqueStrings(heartbeats.map(({ heartbeatId }) => heartbeatId), "before heartbeat ids")
  for (const oldId of [identity.relayRegistrationId, identity.runtimeMachineId]) {
    if (!heartbeats.some((item) => item.identity === oldId)) fail("old_heartbeats_missing", "before relay evidence omits the old relay or runtime identity")
  }
  return { info, allocation: resource, identity, services, processes, states, cloudRows, heartbeats }
}

function validateRebuild(rebuild, reviewed, before) {
  const info = captureReceipt(rebuild, "rebuild", {
    fields: ["allocation", "request", "completion"],
    allowed: ["hetzner-api"],
    required: ["hetzner-api"],
  })
  const resource = allocation(rebuild.allocation, "rebuild.allocation")
  sameAllocation(before.allocation, resource, "rebuild allocation")
  if (resource.imageId !== before.allocation.imageId) fail("rebuild_source_image", "rebuild allocation does not identify the pre-rebuild image")
  const request = plainObject(rebuild.request, "rebuild.request", [
    "requestId", "actionId", "serverId", "sourceImageId", "targetImageId", "requestedAt", "destructive",
  ])
  for (const field of ["requestId", "actionId", "serverId", "sourceImageId", "targetImageId"]) nonEmpty(request[field], `rebuild.request.${field}`)
  const requestedAt = timestamp(request.requestedAt, "rebuild.request.requestedAt")
  if (request.serverId !== resource.serverId || request.sourceImageId !== before.allocation.imageId
    || request.targetImageId !== reviewed.approvedImage.imageId || request.destructive !== true) {
    fail("rebuild_request_mismatch", "provider request is not a destructive rebuild of this allocation to the approved image")
  }
  const completion = plainObject(rebuild.completion, "rebuild.completion", [
    "requestId", "actionId", "serverId", "imageId", "status", "completedAt",
  ])
  for (const field of ["requestId", "actionId", "serverId", "imageId", "status"]) nonEmpty(completion[field], `rebuild.completion.${field}`)
  const completedAt = timestamp(completion.completedAt, "rebuild.completion.completedAt")
  if (completion.requestId !== request.requestId || completion.actionId !== request.actionId
    || completion.serverId !== resource.serverId || completion.imageId !== reviewed.approvedImage.imageId
    || completion.status !== "success" || completedAt <= requestedAt || completedAt > info.capturedAt) {
    fail("rebuild_completion_mismatch", "provider rebuild completion is missing, stale, or contradicts its request")
  }
  if (before.info.capturedAt >= requestedAt || reviewed.info.capturedAt >= requestedAt) {
    fail("receipt_stale", "before or reviewed evidence was captured after rebuild began")
  }
  const requestObservation = requireObservation(info, "rebuild-request", "hetzner-api", "rebuild")
  const completionObservation = requireObservation(info, "rebuild-completion", "hetzner-api", "rebuild", requestObservation.capturedAt)
  if (requestObservation.capturedAt > requestedAt || completionObservation.capturedAt < completedAt) {
    fail("rebuild_observation_time", "provider request/completion captures do not bracket the rebuild action")
  }
  return { info, allocation: resource, request, completion, completedAt }
}

function validateExactAbsence(entries, expected, key, label, info, authority, requiredObservationTime, extraKeys = []) {
  if (!Array.isArray(entries)) fail("residue_evidence_missing", `${label} absence observations are missing`)
  const expectedValues = expected.map((item) => typeof item === "string" ? item : item[key])
  const actual = entries.map((item, index) => {
    plainObject(item, `${label} absence ${index}`, [key, "present", "observationId", ...extraKeys])
    const id = nonEmpty(item[key], `${label} absence identity`)
    if (item.present !== false) fail("old_identity_present", `${label} still contains an old identity`)
    const observation = info.observations.get(nonEmpty(item.observationId, `${label} observation id`))
    const requiredAuthority = typeof authority === "function" ? authority(item) : authority
    if (!observation || observation.authority !== requiredAuthority || observation.capturedAt <= requiredObservationTime) {
      fail("residue_evidence_missing", `${label} absence is not backed by a fresh ${requiredAuthority} observation`)
    }
    return id
  })
  uniqueStrings(actual, `${label} absence identities`)
  if (expectedValues.length !== actual.length || expectedValues.some((item) => !actual.includes(item))) {
    fail("residue_evidence_incomplete", `${label} does not account for every old identity exactly once`)
  }
}

function validateAfter(after, reviewed, before, rebuild) {
  const info = captureReceipt(after, "after", {
    fields: ["allocation", "identity", "absence"],
    allowed: ["hetzner-api", "host-command", "cloud-api", "relay-api", "release-verifier"],
    required: ["hetzner-api", "host-command", "cloud-api", "relay-api", "release-verifier"],
  })
  const resource = allocation(after.allocation, "after.allocation")
  for (const observation of info.observations.values()) {
    if (observation.capturedAt <= rebuild.completedAt) fail("receipt_stale", "after evidence contains an observation from before rebuild completion")
  }
  requireCapturedOperation(info, "hetzner-api", ["server"])
  requireCapturedOperation(info, "host-command", ["systemd"])
  requireCapturedOperation(info, "cloud-api", ["enrollment"])
  requireCapturedOperation(info, "relay-api", ["heartbeat"])
  sameAllocation(before.allocation, resource, "after allocation")
  if (resource.imageId !== reviewed.approvedImage.imageId || resource.imageId !== rebuild.request.targetImageId) {
    fail("after_image_mismatch", "rebuilt allocation is not running the approved clean image")
  }
  if (info.capturedAt <= rebuild.completedAt) fail("receipt_stale", "after receipt predates provider rebuild completion")
  const hostObservation = [...info.observations.values()].find((item) => item.authority === "host-command")
  const cloudObservation = [...info.observations.values()].find((item) => item.authority === "cloud-api")
  const relayObservation = [...info.observations.values()].find((item) => item.authority === "relay-api")
  for (const observation of [hostObservation, cloudObservation, relayObservation]) {
    if (observation.capturedAt <= rebuild.completedAt) fail("receipt_stale", "after host or control-plane evidence predates rebuild completion")
  }
  const releaseObservation = [...info.observations.values()].find((item) => item.authority === "release-verifier")
  if (!releaseObservation || !releaseObservation.operation.toLowerCase().includes("verify-image-release")
    || releaseObservation.capturedAt <= rebuild.completedAt) {
    fail("release_evidence_missing", "after receipt lacks fresh signed-release verification")
  }

  const identity = plainObject(after.identity, "after.identity", [
    "bootId", "machineId", "enrollmentId", "relayRegistrationId", "runtimeMachineId", "release",
    "serviceInstances", "processes", "stateInstances",
  ])
  const oldIdentity = before.identity
  const rotated = ["bootId", "machineId", "enrollmentId", "relayRegistrationId", "runtimeMachineId"]
  for (const field of rotated) {
    nonEmpty(identity[field], `after.identity.${field}`, 3)
    if (identity[field] === oldIdentity[field]) fail("identity_not_rotated", `after ${field} matches the old machine identity`)
  }
  const release = validateRelease(identity.release, "after.identity.release", { instance: true, verified: true })
  for (const field of ["digest", "sourceCommit", "sourceTree"]) {
    if (release[field] !== reviewed.release[field]) fail("release_mismatch", `after signed release ${field} differs from the reviewed release`)
  }
  requireObservation(info, releaseObservation.id, "release-verifier", "verify-image-release", rebuild.completedAt)

  if (!Array.isArray(identity.serviceInstances) || identity.serviceInstances.length === 0) {
    fail("new_services_missing", "after receipt must identify new service instances")
  }
  const newServices = identity.serviceInstances.map((item, index) => identityPair(item, `after service ${index}`, ["unit", "invocationId"]))
  uniqueStrings(newServices.map(({ unit }) => unit), "after service units")
  uniqueStrings(newServices.map(({ invocationId }) => invocationId), "after service invocation ids")
  if (!newServices.some(({ unit }) => unit === "chariox-path1-managed-bootstrap.service")
    || newServices.some(({ unit }) => unit === "chariox-managed-bootstrap.service"
      || unit === "chariox-disposable-worker-bootstrap.service")) {
    fail("path1_service_topology", "rebuilt Path-1 managed home must run its dedicated bootstrap service")
  }
  const oldServiceIds = new Set(before.services.map(({ invocationId }) => invocationId))
  if (newServices.some(({ invocationId }) => oldServiceIds.has(invocationId))) {
    fail("identity_not_rotated", "new systemd service list contains an old invocation identity")
  }
  const newProcesses = uniqueStrings(identity.processes, "after process identities")
  if (newProcesses.some((id) => before.processes.includes(id))) fail("identity_not_rotated", "after process list contains an old process identity")
  if (!Array.isArray(identity.stateInstances) || identity.stateInstances.length < before.states.length) {
    fail("new_state_missing", "after receipt must identify all recreated state roots")
  }
  const newStates = identity.stateInstances.map((item, index) => identityPair(item, `after state ${index}`, ["path", "instanceId"]))
  const oldStates = new Map(before.states.map(({ path, instanceId }) => [path, instanceId]))
  uniqueStrings(newStates.map(({ path }) => path), "after state paths")
  uniqueStrings(newStates.map(({ instanceId }) => instanceId), "after state instance ids")
  if ([...oldStates].some(([path, oldId]) => {
    const current = newStates.find((item) => item.path === path)
    return !current || current.instanceId === oldId
  })) {
    fail("identity_not_rotated", "state roots reuse old /var/lib/chariox or /home/chariox identities")
  }
  if (identity.release.instanceId === oldIdentity.release.instanceId) fail("identity_not_rotated", "managed release installation reuses the old release instance")

  const absence = plainObject(after.absence, "after.absence", [
    "identities", "services", "processes", "stateInstances", "releaseInstances", "cloudRows", "heartbeats",
  ])
  const oldIdentityEntries = rotated.map((kind) => ({ kind, value: oldIdentity[kind] }))
  validateExactAbsence(absence.identities, oldIdentityEntries, "value", "old machine and control identities", info, (item) => {
    if (item.kind === "enrollmentId" || item.kind === "runtimeMachineId") return "cloud-api"
    if (item.kind === "relayRegistrationId") return "relay-api"
    return "host-command"
  }, rebuild.completedAt, ["kind"])
  if (absence.identities.length !== oldIdentityEntries.length
    || oldIdentityEntries.some(({ kind, value }) => !absence.identities.some((item) => item.kind === kind && item.value === value))) {
    fail("residue_evidence_incomplete", "old machine and control identity checks are incomplete")
  }
  for (const item of absence.identities) {
    const kind = item.kind
    const expectedAuthority = kind === "enrollmentId" || kind === "runtimeMachineId" ? "cloud-api"
      : kind === "relayRegistrationId" ? "relay-api" : "host-command"
    const observation = info.observations.get(item.observationId)
    if (!observation || observation.authority !== expectedAuthority) {
      fail("residue_evidence_missing", `old ${kind} absence lacks the authoritative source`)
    }
  }
  validateExactAbsence(absence.services, before.services.map(({ invocationId }) => invocationId), "invocationId", "old services", info, "host-command", rebuild.completedAt)
  validateExactAbsence(absence.processes, before.processes, "processId", "old processes", info, "host-command", rebuild.completedAt)
  validateExactAbsence(absence.stateInstances, before.states.map(({ instanceId }) => instanceId), "instanceId", "old state", info, "host-command", rebuild.completedAt)
  validateExactAbsence(absence.releaseInstances, [oldIdentity.release.instanceId], "instanceId", "old release", info, "host-command", rebuild.completedAt)
  validateExactAbsence(absence.cloudRows, before.cloudRows.map(({ rowId }) => rowId), "rowId", "old Cloud rows", info, "cloud-api", rebuild.completedAt)
  validateExactAbsence(absence.heartbeats, before.heartbeats.map(({ heartbeatId }) => heartbeatId), "heartbeatId", "old relay heartbeats", info, "relay-api", rebuild.completedAt)
  return { info, allocation: resource, identity, release, rotated, absence }
}

function validateCleanup(cleanup, before, after) {
  const info = captureReceipt(cleanup, "cleanup", {
    fields: ["allocation", "cleanup"],
    allowed: ["hetzner-api", "host-command", "cloud-api", "relay-api", "cleanup-command"],
    required: ["hetzner-api", "host-command", "cloud-api", "relay-api", "cleanup-command"],
  })
  const resource = allocation(cleanup.allocation, "cleanup.allocation")
  for (const observation of info.observations.values()) {
    if (observation.capturedAt <= after.info.capturedAt) fail("cleanup_stale", "cleanup evidence contains an observation from before the post-rebuild scan")
  }
  requireCapturedOperation(info, "cleanup-command", ["cleanup"])
  requireCapturedOperation(info, "hetzner-api", ["server"])
  requireCapturedOperation(info, "host-command", ["cleanup"])
  requireCapturedOperation(info, "cloud-api", ["cleanup"])
  requireCapturedOperation(info, "relay-api", ["cleanup"])
  sameAllocation(after.allocation, resource, "cleanup allocation", true)
  sameAllocation(before.allocation, resource, "cleanup allocation")
  const record = plainObject(cleanup.cleanup, "cleanup.cleanup", [
    "recordId", "completedAt", "oldRuntimeRetired", "rebuiltServerPreserved", "unrelatedResourcesUntouched",
    "ownedTemporaryArtifactsRemoved", "remainingOldIdentities",
  ])
  nonEmpty(record.recordId, "cleanup record id", 3)
  const completedAt = timestamp(record.completedAt, "cleanup.completedAt")
  if (completedAt <= after.info.capturedAt || completedAt > info.capturedAt) {
    fail("cleanup_stale", "cleanup receipt is stale or predates the post-rebuild residue scan")
  }
  for (const field of ["oldRuntimeRetired", "rebuiltServerPreserved", "unrelatedResourcesUntouched", "ownedTemporaryArtifactsRemoved"]) {
    if (record[field] !== true) fail("cleanup_incomplete", `cleanup does not confirm ${field}`)
  }
  if (!Array.isArray(record.remainingOldIdentities) || record.remainingOldIdentities.length !== 0) {
    fail("cleanup_incomplete", "cleanup records old runtime identities or residue still present")
  }
  const observation = requireObservation(info, "cleanup-complete", "cleanup-command", "cleanup", after.info.capturedAt)
  if (observation.capturedAt < completedAt) fail("cleanup_observation_time", "cleanup evidence predates the cleanup completion record")
  return { info, allocation: resource, record }
}

export function verifyPath1RebuildEvidence(receipts) {
  plainObject(receipts, "evidence input", RECEIPT_KINDS)
  const { reviewed, before, rebuild, after, cleanup } = receipts
  for (const [kind, receipt] of Object.entries({ reviewed, before, rebuild, after, cleanup })) {
    if (!receipt || receipt.kind !== kind) fail("receipt_missing", `required ${kind} receipt is missing`)
  }
  const validatedReviewed = validateReviewed(reviewed)
  const validatedBefore = validateBefore(before)
  const validatedRebuild = validateRebuild(rebuild, validatedReviewed, validatedBefore)
  const validatedAfter = validateAfter(after, validatedReviewed, validatedBefore, validatedRebuild)
  const validatedCleanup = validateCleanup(cleanup, validatedBefore, validatedAfter)
  const campaignIds = [
    validatedReviewed.info.campaignId,
    validatedBefore.info.campaignId,
    validatedRebuild.info.campaignId,
    validatedAfter.info.campaignId,
    validatedCleanup.info.campaignId,
  ]
  if (new Set(campaignIds).size !== 1) fail("campaign_mismatch", "receipts do not belong to one rebuild campaign")
  return {
    schema: REPORT_SCHEMA,
    status: "pass",
    campaignId: campaignIds[0],
    allocation: {
      provider: validatedAfter.allocation.provider,
      projectId: validatedAfter.allocation.projectId,
      serverId: validatedAfter.allocation.serverId,
      serverType: validatedAfter.allocation.serverType,
      datacenter: validatedAfter.allocation.datacenter,
    },
    approvedImage: {
      imageId: validatedReviewed.approvedImage.imageId,
      name: validatedReviewed.approvedImage.name,
      type: validatedReviewed.approvedImage.type,
    },
    reviewedRelease: {
      digest: validatedReviewed.release.digest,
      sourceCommit: validatedReviewed.release.sourceCommit,
      sourceTree: validatedReviewed.release.sourceTree,
    },
    newIdentities: Object.fromEntries(validatedAfter.rotated.map((field) => [field, validatedAfter.identity[field]])),
    retiredIdentityCounts: {
      serviceInstances: validatedBefore.services.length,
      processInstances: validatedBefore.processes.length,
      stateRoots: validatedBefore.states.length,
      releaseInstances: 1,
      cloudRows: validatedBefore.cloudRows.length,
      relayHeartbeats: validatedBefore.heartbeats.length,
    },
    cleanup: {
      recordId: validatedCleanup.record.recordId,
      completedAt: validatedCleanup.record.completedAt,
      rebuiltServerPreserved: validatedCleanup.record.rebuiltServerPreserved,
      unrelatedResourcesUntouched: validatedCleanup.record.unrelatedResourcesUntouched,
      ownedTemporaryArtifactsRemoved: validatedCleanup.record.ownedTemporaryArtifactsRemoved,
    },
    evidenceRefs: Object.fromEntries(Object.entries({ reviewed, before, rebuild, after, cleanup }).map(([kind, receipt]) => [
      kind,
      receipt.observations.map(({ authority, evidenceRef, evidenceSha256 }) => ({ authority, evidenceRef, evidenceSha256 })),
    ])),
  }
}

export async function readEvidenceReceipt(path, expectedKind) {
  const absolute = resolve(path)
  const metadata = await lstat(absolute).catch((error) => fail("receipt_unreadable", `${expectedKind} receipt cannot be read: ${error.message}`))
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > MAX_RECEIPT_BYTES) {
    fail("receipt_unreadable", `${expectedKind} receipt must be a bounded regular file`)
  }
  let receipt
  try {
    receipt = JSON.parse(await readFile(absolute, "utf8"))
  } catch {
    fail("receipt_json", `${expectedKind} receipt is not valid JSON`)
  }
  if (!receipt || receipt.kind !== expectedKind) fail("receipt_kind", `${expectedKind} receipt has the wrong kind`)
  return receipt
}

export async function verifyEvidenceArtifactFiles(receipts) {
  for (const kind of RECEIPT_KINDS) {
    for (const observation of receipts[kind].observations) {
      const path = resolve(observation.evidenceRef.slice("file:".length))
      const metadata = await lstat(path).catch((error) => fail("evidence_artifact_missing", `${kind} capture artifact cannot be read: ${error.message}`))
      if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > MAX_EVIDENCE_BYTES) {
        fail("evidence_artifact_invalid", `${kind} capture artifact must be a bounded regular file`)
      }
      const bytes = await readFile(path)
      const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
      if (digest !== observation.evidenceSha256) fail("evidence_artifact_mismatch", `${kind} capture artifact digest does not match its receipt`)
    }
  }
}

function parseArgs(args) {
  const flags = new Map([
    ["--reviewed", "reviewed"], ["--before", "before"], ["--rebuild", "rebuild"],
    ["--after", "after"], ["--cleanup", "cleanup"], ["--report", "report"],
  ])
  const values = new Map()
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index]
    const key = flags.get(flag)
    const value = args[index + 1]
    if (!key || value === undefined || value.startsWith("--") || values.has(key)) {
      fail("usage", "supply each required flag once with a value")
    }
    values.set(key, value)
  }
  const missing = [...flags.values()].filter((key) => !values.has(key))
  if (missing.length) fail("usage", `missing required inputs: ${missing.map((key) => `--${key}`).join(", ")}`)
  return values
}

async function main() {
  let reportPath
  try {
    const args = parseArgs(process.argv.slice(2))
    reportPath = resolve(args.get("report"))
    const inputs = new Map()
    const paths = new Set([reportPath])
    for (const kind of RECEIPT_KINDS) {
      const path = resolve(args.get(kind === "reviewed" ? "reviewed" : kind))
      if (paths.has(path)) fail("receipt_paths", "receipt and report paths must be distinct")
      paths.add(path)
      inputs.set(kind, await readEvidenceReceipt(path, kind))
    }
    const report = verifyPath1RebuildEvidence(Object.fromEntries(inputs))
    await verifyEvidenceArtifactFiles(Object.fromEntries(inputs))
    await writeReport(reportPath, report)
    process.stdout.write(`PASS Path-1 fresh-equivalent rebuild evidence for Hetzner server ${report.allocation.serverId}\n`)
  } catch (error) {
    const code = error instanceof RebuildEvidenceError ? error.code : "verifier_error"
    const message = error instanceof Error ? error.message : String(error)
    if (reportPath) {
      try {
        await writeReport(reportPath, { schema: REPORT_SCHEMA, status: "fail", error: { code, message } })
      } catch (writeError) {
        process.stderr.write(`FAIL ${code}: ${message}; failure report could not be written: ${writeError.message}\n`)
        process.exitCode = 1
        return
      }
    }
    process.stderr.write(`FAIL ${code}: ${message}\n`)
    process.exitCode = 1
  }
}

async function writeReport(path, report) {
  await writeFile(path, `${JSON.stringify(report, null, 2)}\n`, { flag: "wx", mode: 0o600 })
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await main()
}

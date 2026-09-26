// Correlate allowlisted Path-1 provider observations with a finalized Cloud capture.
// This is a pure evidence join, not an MP acceptance verdict.

export const PROVIDER_CLOUD_CORRELATION_SCHEMA = "chariox.path1-provider-cloud-correlation/v1"

const PROVIDER_SCHEMA = "chariox.path1-provider-rebuild-capture/v1"
const CLOUD_SCHEMA = "chariox.path1-cloud-reimage-capture/v1"
const NUMERIC_ID = /^[1-9][0-9]{0,15}$/
const CLOUD_ID = /^[a-zA-Z0-9][a-zA-Z0-9._:/ -]{0,255}$/
const DIGEST = /^sha256:[a-f0-9]{64}$/
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/
const MACHINE_ID = /^[a-f0-9]{32}$/

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function record(value, label) {
  requireValue(value && typeof value === "object" && !Array.isArray(value)
    && [Object.prototype, null].includes(Object.getPrototypeOf(value)), `${label} is invalid`)
  return value
}

function numericId(value, label) {
  requireValue(typeof value === "string" && NUMERIC_ID.test(value)
    && Number.isSafeInteger(Number(value)), `${label} is missing or invalid`)
  return value
}

function timestamp(value, label) {
  requireValue(typeof value === "string" && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)
    && Number.isFinite(Date.parse(value)) && new Date(value).toISOString() === value,
  `${label} must be a canonical UTC timestamp`)
  const milliseconds = Date.parse(value)
  return { value, nanoseconds: BigInt(milliseconds) * 1_000_000n }
}

function providerTimestamp(value, label) {
  const match = typeof value === "string"
    ? /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,9}))?Z$/.exec(value)
    : null
  requireValue(match, `${label} is missing or invalid`)
  const millis = (match[2] ?? "").slice(0, 3).padEnd(3, "0")
  const normalized = `${match[1]}.${millis}Z`
  const milliseconds = Date.parse(normalized)
  requireValue(Number.isFinite(milliseconds) && new Date(milliseconds).toISOString() === normalized,
    `${label} is missing or invalid`)
  const subMillisecond = (match[2] ?? "").slice(3).padEnd(6, "0") || "0"
  return { value, nanoseconds: BigInt(milliseconds) * 1_000_000n + BigInt(subMillisecond) }
}

function providerCapture(value, kind) {
  record(value, `${kind} provider capture`)
  requireValue(value.schema === PROVIDER_SCHEMA && value.provider === "hetzner-cloud"
    && value.kind === kind, `${kind} provider capture has the wrong schema or kind`)
  return timestamp(value.capturedAt, `${kind} provider capture time`)
}

function serverIdentity(value, label) {
  record(value, label)
  return {
    serverId: numericId(value.serverId, `${label} server id`),
    imageId: numericId(value.imageId, `${label} image id`),
    serverTypeId: numericId(value.serverTypeId, `${label} server type id`),
    locationId: numericId(value.locationId, `${label} location id`),
    datacenterId: numericId(value.datacenterId, `${label} datacenter id`),
  }
}

function cloudCapture(value) {
  record(value, "Cloud capture")
  requireValue(value.schema === CLOUD_SCHEMA, "unsupported Cloud capture schema")
  requireValue(Number.isSafeInteger(value.minimumProtocolVersion) && value.minimumProtocolVersion > 0
    && typeof value.environmentId === "string" && CLOUD_ID.test(value.environmentId)
    && typeof value.operationId === "string" && CLOUD_ID.test(value.operationId)
    && typeof value.receiptId === "string" && CLOUD_ID.test(value.receiptId)
    && DIGEST.test(value.receiptDigest ?? "")
    && Number.isSafeInteger(value.generation) && value.generation > 1
    && value.previousGeneration === value.generation - 1,
  "Cloud capture lacks its owner-bound finalized receipt identity")
  const providerServerId = numericId(value.providerServerId, "Cloud provider server id")
  const providerImageId = numericId(value.providerImageId, "Cloud target image id")
  const rebuildActionId = numericId(value.rebuildActionId, "Cloud rebuild action id")
  const before = record(value.before, "Cloud retained baseline")
  const after = record(value.after, "Cloud new identity")
  requireValue(UUID.test(before.bootId ?? "") && MACHINE_ID.test(before.machineId ?? "")
    && UUID.test(after.bootId ?? "") && MACHINE_ID.test(after.machineId ?? "")
    && before.bootId !== after.bootId && before.machineId !== after.machineId,
  "Cloud capture lacks rotated host identities")
  const baselineAt = timestamp(before.observedAt, "Cloud retained baseline time")
  const requestedAt = timestamp(value.requestedAt, "Cloud requestedAt")
  const completedAt = timestamp(value.completedAt, "Cloud completedAt")
  const capturedAt = timestamp(value.capturedAt, "Cloud capture time")
  requireValue(baselineAt.nanoseconds <= requestedAt.nanoseconds
    && requestedAt.nanoseconds <= completedAt.nanoseconds
    && completedAt.nanoseconds <= capturedAt.nanoseconds,
  "Cloud baseline, requested, and completed times are out of order")
  return {
    environmentId: value.environmentId,
    operationId: value.operationId,
    receiptId: value.receiptId,
    receiptDigest: value.receiptDigest,
    generation: value.generation,
    previousGeneration: value.previousGeneration,
    providerServerId,
    providerImageId,
    rebuildActionId,
    requestedAt,
    completedAt,
    capturedAt,
  }
}

function rebuildAction(value, expectedId) {
  record(value, "provider action")
  requireValue(numericId(value.actionId, "provider action id") === expectedId,
    "provider action id does not match the Cloud operation")
  requireValue(value.command === "rebuild", "provider action is not a rebuild")
  requireValue(value.status === "success", "provider rebuild action is not successful")
  requireValue(Object.hasOwn(value, "error") && value.error === null,
    "provider rebuild action error is not null")
  const started = providerTimestamp(value.started, "provider action start time")
  const finished = providerTimestamp(value.finished, "provider action finish time")
  return {
    observation: {
      actionId: value.actionId,
      command: value.command,
      status: value.status,
      error: null,
      started: started.value,
      finished: finished.value,
    },
    started,
    finished,
  }
}

function sameAllocation(source, target) {
  return source.serverId === target.serverId
    && source.serverTypeId === target.serverTypeId
    && source.locationId === target.locationId
    && source.datacenterId === target.datacenterId
}

export function correlatePath1ProviderCloud({ before, after, cloud }) {
  const preparedAt = providerCapture(before, "before")
  const afterCapturedAt = providerCapture(after, "after")
  const cloudEvidence = cloudCapture(cloud)
  const source = serverIdentity(before.server, "before provider server")
  const afterBefore = serverIdentity(after.serverBefore, "after provider pre-history server")
  const target = serverIdentity(after.serverAfter, "after provider post-history server")
  const action = rebuildAction(after.action, cloudEvidence.rebuildActionId)
  const requestedAt = cloudEvidence.requestedAt
  const completedAt = cloudEvidence.completedAt
  const afterRequestedAt = timestamp(after.requestedAt, "provider after requestedAt")

  requireValue(source.serverId === cloudEvidence.providerServerId
    && afterBefore.serverId === cloudEvidence.providerServerId
    && target.serverId === cloudEvidence.providerServerId,
  "provider captures do not identify the same Cloud-bound server")
  requireValue(source.imageId !== cloudEvidence.providerImageId,
    "source and target image identities did not change")
  requireValue(afterBefore.imageId === cloudEvidence.providerImageId
    && target.imageId === cloudEvidence.providerImageId,
  "provider after capture does not match the Cloud target image")
  requireValue(sameAllocation(source, afterBefore) && sameAllocation(source, target),
    "provider allocation server type, location, or datacenter changed")
  requireValue(afterRequestedAt.nanoseconds === requestedAt.nanoseconds,
    "provider and Cloud requestedAt values do not match")
  requireValue(preparedAt.nanoseconds <= requestedAt.nanoseconds
    && requestedAt.nanoseconds <= action.started.nanoseconds
    && action.started.nanoseconds <= action.finished.nanoseconds
    && action.finished.nanoseconds <= completedAt.nanoseconds
    && completedAt.nanoseconds <= afterCapturedAt.nanoseconds,
  "provider preparation, request, action, completion, and after capture times are out of order")

  return {
    schema: PROVIDER_CLOUD_CORRELATION_SCHEMA,
    cloud: {
      environmentId: cloudEvidence.environmentId,
      operationId: cloudEvidence.operationId,
      receiptId: cloudEvidence.receiptId,
      receiptDigest: cloudEvidence.receiptDigest,
      generation: cloudEvidence.generation,
      previousGeneration: cloudEvidence.previousGeneration,
    },
    provider: {
      serverId: source.serverId,
      sourceImageId: source.imageId,
      targetImageId: cloudEvidence.providerImageId,
      serverTypeId: source.serverTypeId,
      locationId: source.locationId,
      datacenterId: source.datacenterId,
      action: action.observation,
    },
    times: {
      // The Cloud schema has no preparedAt field; use the before provider capture as that observation.
      providerPreparedAt: preparedAt.value,
      requestedAt: requestedAt.value,
      actionStartedAt: action.started.value,
      actionFinishedAt: action.finished.value,
      completedAt: completedAt.value,
      cloudCapturedAt: cloudEvidence.capturedAt.value,
      providerAfterCapturedAt: afterCapturedAt.value,
    },
    providerProjectProvenance: {
      status: "unexecuted",
      reason: "hetzner_captures_do_not_expose_project_identity",
    },
  }
}

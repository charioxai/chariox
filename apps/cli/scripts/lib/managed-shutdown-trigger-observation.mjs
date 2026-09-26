import { nonEmpty, requireValue, SHUTDOWN_TRIGGER_LIMITS } from "./managed-shutdown-trigger-config.mjs"

const DESIRED_STATES = new Set(["running", "stopped", "deleted"])
const OBSERVED_STATES = new Set([
  "requested", "provisioning", "bootstrapping", "awaiting_context", "ready", "starting",
  "stopping", "stopped", "deleting", "deleted", "failed",
])
const OPERATION_KINDS = new Set(["create", "start", "stop", "restart", "delete", "reimage"])
const OPERATION_STATUSES = new Set(["pending", "running", "succeeded", "failed"])

export function timestamp(value, label, nullable = false) {
  if (nullable && value === null) return null
  requireValue(typeof value === "string" && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value)
    && Number.isFinite(Date.parse(value)) && new Date(value).toISOString() === value,
  `${label} is unavailable or invalid`)
  return value
}

export function projectSummary(value) {
  requireValue(value && typeof value === "object" && !Array.isArray(value), "managed environment summary is unavailable")
  requireValue(DESIRED_STATES.has(value.desiredState) && OBSERVED_STATES.has(value.observedState),
    "managed environment state is unavailable")
  requireValue(Number.isSafeInteger(value.desiredRevision) && value.desiredRevision >= 0
    && Number.isSafeInteger(value.observedRevision) && value.observedRevision >= 0,
  "managed environment revision is unavailable")
  const policy = value.autoStopPolicy
  requireValue(policy && Number.isSafeInteger(policy.minimumRuntimeSeconds) && policy.minimumRuntimeSeconds >= 0
    && (policy.idleDelaySeconds === null || (Number.isSafeInteger(policy.idleDelaySeconds) && policy.idleDelaySeconds >= 0)),
  "managed environment policy is unavailable")
  const result = {
    environmentId: nonEmpty(value.environmentId, "environment identity"),
    accountId: nonEmpty(value.accountId, "account identity"),
    createdByUserId: nonEmpty(value.createdByUserId, "owner identity"),
    name: nonEmpty(value.name, "environment name"),
    region: nonEmpty(value.region, "environment region"),
    computeClass: nonEmpty(value.computeClass, "environment compute class"),
    desiredState: value.desiredState,
    observedState: value.observedState,
    desiredRevision: value.desiredRevision,
    observedRevision: value.observedRevision,
    autoStopPolicy: {
      minimumRuntimeSeconds: policy.minimumRuntimeSeconds,
      idleDelaySeconds: policy.idleDelaySeconds,
    },
    createdAt: timestamp(value.createdAt, "environment creation time"),
    updatedAt: timestamp(value.updatedAt, "environment update time"),
  }
  for (const key of ["runtimeMachineId", "runtimeKernelId", "runningAgentCount", "lastActivityReportedAt",
    "lastActivityChangedAt", "autoStopWarningAt", "autoStopDeadlineAt"]) {
    if (!Object.hasOwn(value, key)) continue
    const field = value[key]
    if (key === "runningAgentCount") {
      requireValue(field === null || field === 0 || field === 1, "managed activity count is invalid")
      result[key] = field
    } else if (key === "runtimeMachineId" || key === "runtimeKernelId") {
      result[key] = field === null ? null : nonEmpty(field, key)
    } else {
      result[key] = timestamp(field, key, true)
    }
  }
  return result
}

export function projectOperation(value) {
  requireValue(value && typeof value === "object" && !Array.isArray(value), "managed operation is unavailable")
  requireValue(OPERATION_KINDS.has(value.kind) && OPERATION_STATUSES.has(value.status)
    && Number.isSafeInteger(value.desiredRevision) && value.desiredRevision > 0
    && Number.isSafeInteger(value.attempt) && value.attempt >= 0,
  "managed operation fields are invalid")
  return {
    operationId: nonEmpty(value.operationId, "operation identity"),
    environmentId: nonEmpty(value.environmentId, "operation environment identity"),
    requestedByUserId: nonEmpty(value.requestedByUserId, "operation actor identity"),
    kind: value.kind,
    desiredRevision: value.desiredRevision,
    status: value.status,
    attempt: value.attempt,
    completedAt: timestamp(value.completedAt, "operation completion time", true),
    createdAt: timestamp(value.createdAt, "operation creation time"),
    updatedAt: timestamp(value.updatedAt, "operation update time"),
  }
}

export function responseBody(response, variant) {
  const body = response?.[variant]
  requireValue(body && typeof body === "object" && !Array.isArray(body), `kernel response ${variant} is unavailable`)
  return body
}

export function assertTargetIdentity(summary, target) {
  requireValue(summary.environmentId === target.environmentId && summary.accountId === target.accountId
    && summary.createdByUserId === target.createdByUserId && summary.name === target.name
    && summary.region === target.region && summary.computeClass === target.computeClass,
  "managed target identity changed")
  for (const key of ["runtimeMachineId", "runtimeKernelId"]) {
    if (target[key] !== null && target[key] !== undefined) {
      requireValue(summary[key] === target[key], "managed runtime identity changed")
    }
  }
}

export function operationList(details, target) {
  if (!Object.hasOwn(details, "operations")) return undefined
  requireValue(Array.isArray(details.operations) && details.operations.length <= SHUTDOWN_TRIGGER_LIMITS.maximumOperations,
    "managed operation history is invalid")
  const operations = details.operations.map(projectOperation)
  const ids = new Set()
  for (const operation of operations) {
    requireValue(operation.environmentId === target.environmentId
      && operation.requestedByUserId === target.createdByUserId
      && !ids.has(operation.operationId),
    "managed operation history is not uniquely owner-bound")
    ids.add(operation.operationId)
  }
  return operations
}

export function recordOperation(capture, operation) {
  const current = capture.operations.findIndex((item) => item.operationId === operation.operationId)
  if (current < 0) capture.operations.push(operation)
  else capture.operations[current] = operation
  requireValue(capture.operations.length <= SHUTDOWN_TRIGGER_LIMITS.maximumOperations,
    "managed operation evidence exceeded the bound")
}

export function recordSummary(capture, summary, capturedAt, force = false) {
  const previous = capture.observations.at(-1)?.environment
  const relevant = {
    desiredState: summary.desiredState,
    observedState: summary.observedState,
    desiredRevision: summary.desiredRevision,
    observedRevision: summary.observedRevision,
    runningAgentCount: summary.runningAgentCount,
    lastActivityChangedAt: summary.lastActivityChangedAt,
    autoStopWarningAt: summary.autoStopWarningAt,
    autoStopDeadlineAt: summary.autoStopDeadlineAt,
  }
  if (!force && previous && JSON.stringify(relevant) === JSON.stringify({
    desiredState: previous.desiredState,
    observedState: previous.observedState,
    desiredRevision: previous.desiredRevision,
    observedRevision: previous.observedRevision,
    runningAgentCount: previous.runningAgentCount,
    lastActivityChangedAt: previous.lastActivityChangedAt,
    autoStopWarningAt: previous.autoStopWarningAt,
    autoStopDeadlineAt: previous.autoStopDeadlineAt,
  })) return
  requireValue(capture.observations.length < SHUTDOWN_TRIGGER_LIMITS.maximumObservations,
    "managed state observation limit reached")
  capture.observations.push({ capturedAt, environment: summary })
}

export function exactOperation(operations, expected, targetId) {
  requireValue(Array.isArray(operations), "managed operation history is unavailable")
  const matches = operations.filter(({ operationId }) => operationId === expected.operationId)
  requireValue(matches.length === 1, "exact managed operation is unavailable or ambiguous")
  const operation = matches[0]
  requireValue(operation.environmentId === targetId && operation.kind === expected.kind
    && operation.desiredRevision === expected.desiredRevision,
  "managed operation identity or revision changed")
  if (operation.status === "failed") throw new Error("managed operation failed")
  if (operation.status === "succeeded") {
    requireValue(typeof operation.completedAt === "string", "managed operation completion receipt is missing")
    requireValue(Date.parse(operation.completedAt) >= Date.parse(operation.createdAt)
      && Date.parse(operation.updatedAt) >= Date.parse(operation.completedAt),
    "managed operation times are inconsistent")
  }
  return operation
}

export function verifyIdleDeadline(summary) {
  const idleAt = timestamp(summary.lastActivityChangedAt, "last-agent-finished time")
  const delay = summary.autoStopPolicy.idleDelaySeconds
  requireValue(delay !== null && delay >= 300, "idle shutdown deadline cannot be derived from activity evidence")
  const deadline = timestamp(summary.autoStopDeadlineAt, "auto-stop deadline")
  requireValue(Date.parse(deadline) >= Date.parse(idleAt) + delay * 1_000
    && summary.autoStopWarningAt === new Date(Date.parse(deadline) - 300_000).toISOString(),
  "Cloud deadline is inconsistent with the signed last-agent-finished time")
  return deadline
}

export function requireOperationHistory(snapshot) {
  requireValue(Array.isArray(snapshot.operations), "managed operation history is unavailable")
  return snapshot.operations
}

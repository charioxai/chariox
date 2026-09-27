import { randomUUID } from "node:crypto"

import {
  assertTargetIdentity,
  exactOperation,
  operationList,
  projectOperation,
  projectSummary,
  recordOperation,
  recordSummary,
  requireOperationHistory,
  responseBody,
} from "./managed-shutdown-trigger-observation.mjs"
import { requireValue, SHUTDOWN_TRIGGER_LIMITS } from "./managed-shutdown-trigger-config.mjs"

export function createManagedShutdownProduct({ deps, options, capture, getTarget, isCleanupStarted,
  waitForAction, remaining, now, pause, runId }) {
  const send = deps.send ?? ((request) => deps.client.send(request))

  const sendUntil = async (request, deadline, preserveMutation = false) => {
    requireValue(isCleanupStarted() || !deps.signal?.aborted, "capture interrupted")
    const milliseconds = remaining(deadline)
    requireValue(milliseconds > 0, "managed shutdown time bound expired")
    let timer
    try {
      const boundedRequest = Promise.race([
        send(request),
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error("managed request time bound expired")), milliseconds)
          timer.unref?.()
        }),
      ])
      return await (preserveMutation || isCleanupStarted() ? boundedRequest : waitForAction(boundedRequest))
    } finally { clearTimeout(timer) }
  }

  const snapshot = async (deadline, forceObservation = false) => {
    const target = getTarget()
    requireValue(target, "managed target is unavailable")
    const response = await sendUntil(deps.requests.getManagedEnvironmentRequest(target.environmentId), deadline)
    const details = responseBody(response, "ManagedEnvironment")
    const summary = projectSummary(details.environment)
    assertTargetIdentity(summary, target)
    requireValue(JSON.stringify(summary.autoStopPolicy) === JSON.stringify(options.descriptor.policy),
      "managed shutdown policy changed")
    const operations = operationList(details, target)
    recordSummary(capture, summary, now().toISOString(), forceObservation)
    if (operations) for (const operation of operations) recordOperation(capture, operation)
    return { summary, operations }
  }

  const poll = async (predicate, deadline, limit = 2_000, finalReadDeadline = deadline) => {
    for (let attempt = 0; attempt < limit && remaining(deadline) > 0; attempt += 1) {
      const current = await snapshot(deadline)
      if (predicate(current)) return current
      await waitForAction(pause(Math.min(SHUTDOWN_TRIGGER_LIMITS.pollMs,
        Math.max(0, remaining(deadline)))))
    }

    // Sample once after the observation window, within the remaining action budget.
    if (remaining(deadline) <= 0 && remaining(finalReadDeadline) > 0) {
      const final = await snapshot(finalReadDeadline, true)
      if (predicate(final)) return final
    }
    throw new Error("managed shutdown observation did not arrive within its time bound")
  }

  const waitForExactOperation = (expected, statePredicate, deadline) => poll((current) => {
    const target = getTarget()
    const operation = exactOperation(requireOperationHistory(current), expected, target.environmentId)
    return operation.status === "succeeded" && typeof operation.completedAt === "string"
      && current.summary.desiredRevision === expected.desiredRevision
      && current.summary.observedRevision === expected.desiredRevision
      && statePredicate(current.summary)
  }, deadline)

  const lifecycle = async (action, deadline) => {
    const target = getTarget()
    const before = await snapshot(deadline)
    const response = await sendUntil(deps.requests.requestManagedEnvironmentLifecycleRequest({
      environmentId: target.environmentId,
      action,
      idempotencyKey: `mp09-${runId}-${action}-${randomUUID()}`,
    }), deadline, true)
    const result = responseBody(response, "ManagedEnvironmentLifecycleRequested").result
    requireValue(result && typeof result === "object", "managed lifecycle response is unavailable")
    const returnedEnvironment = projectSummary(result.environment)
    assertTargetIdentity(returnedEnvironment, target)
    const operation = projectOperation(result.operation)
    requireValue(operation.environmentId === target.environmentId
      && operation.requestedByUserId === target.createdByUserId && operation.kind === action
      && operation.desiredRevision > before.summary.desiredRevision,
    "managed lifecycle operation does not match the requested action")
    requireValue(Date.parse(operation.createdAt) >= Date.parse(before.summary.updatedAt),
      "managed lifecycle operation predates its owner snapshot")
    recordOperation(capture, operation)
    return operation
  }

  return { sendUntil, snapshot, poll, waitForExactOperation, lifecycle }
}

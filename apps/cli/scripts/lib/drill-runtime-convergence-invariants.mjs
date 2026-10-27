import {
  DRILL_CHAOS_INVARIANT_IDS,
  DRILL_CHAOS_INVARIANTS_SCHEMA,
  validateDrillChaosInvariantReport,
} from "./drill-chaos-contract.mjs"

export function evaluateDrillRuntimeConvergenceInvariants({
  acceptedOperations,
  executionCounts,
  kernelState,
  clientStates,
  cursorHistory,
  authorityMutations,
  queue,
  resources,
  staleCallbacks,
}) {
  const accepted = [...acceptedOperations]
  const acceptedSet = new Set(accepted)
  const counts = normalizeCounts(executionCounts)
  const normalizedKernel = normalizeState(kernelState)
  const normalizedClients = Object.fromEntries(Object.entries(clientStates)
    .map(([clientId, state]) => [clientId, normalizeState(state)]))
  const lost = accepted.filter((operationId) => (counts[operationId] ?? 0) === 0)
  const unexpected = Object.entries(counts)
    .filter(([operationId, count]) => count > 0 && !acceptedSet.has(operationId))
    .map(([operationId, count]) => ({ operationId, count }))
  const duplicated = Object.entries(counts)
    .filter(([, count]) => count > 1)
    .map(([operationId, count]) => ({ operationId, count }))
  const cursorViolations = monotonicCursorViolations(cursorHistory, normalizedKernel.cursor)
  const divergentClients = Object.entries(normalizedClients)
    .filter(([, state]) => canonicalJson(state) !== canonicalJson(normalizedKernel))
    .map(([clientId, state]) => ({ clientId, cursor: state.cursor, operationIds: state.operationIds }))
  const invalidAuthority = authorityMutations.filter((mutation) => (
    mutation.owner !== "kernel-authority" || !acceptedSet.has(mutation.operationId)
  ))
  const pendingResources = Object.entries(resources).filter(([, count]) => count !== 0)
  const checks = [
    check("no-action-loss", lost.length === 0 && unexpected.length === 0, "Every accepted operation executes and no unaccepted operation runs.", {
      acceptedOperations: accepted,
      lostOperations: lost,
      unexpectedOperations: unexpected,
    }),
    check("no-duplicate-execution", duplicated.length === 0, "Idempotency prevents duplicate authoritative execution.", {
      executionCounts: counts,
      duplicatedOperations: duplicated,
    }),
    check("monotonic-cursors", cursorViolations.length === 0, "Client cursors are monotonic and never exceed kernel authority.", {
      kernelCursor: normalizedKernel.cursor,
      cursorHistory,
      violations: cursorViolations,
    }),
    check("valid-authority", invalidAuthority.length === 0, "Only kernel authority applies accepted operations to canonical runtime state.", {
      mutationCount: authorityMutations.length,
      invalidMutations: invalidAuthority,
    }),
    check("eventual-client-convergence", divergentClients.length === 0, "All client projections converge to the canonical kernel state.", {
      kernelState: normalizedKernel,
      divergentClients,
    }),
    check("bounded-queues", queue.maxObserved <= queue.limit && queue.pending === 0, "Fault queues remain bounded and drain fully.", {
      ...queue,
    }),
    check("resource-cleanup", pendingResources.length === 0, "All run-owned virtual resources are released.", {
      resources,
      pendingResources,
    }),
    check(
      "stale-callback-suppression",
      staleCallbacks.executed === 0 && staleCallbacks.suppressed === staleCallbacks.scheduled,
      "Callbacks from dead process generations never mutate recovered runtime state.",
      staleCallbacks,
    ),
  ].sort((left, right) => left.id.localeCompare(right.id))
  if (JSON.stringify(checks.map((item) => item.id)) !== JSON.stringify(DRILL_CHAOS_INVARIANT_IDS)) {
    throw new Error("runtime convergence evaluator does not cover the stable chaos invariant contract")
  }
  return validateDrillChaosInvariantReport({
    schema: DRILL_CHAOS_INVARIANTS_SCHEMA,
    status: checks.every((item) => item.ok) ? "passed" : "failed",
    checks,
  })
}

export function assertDrillRuntimeConvergenceInvariants(report) {
  validateDrillChaosInvariantReport(report)
  const failed = report.checks.filter((checkItem) => !checkItem.ok)
  if (failed.length > 0) {
    throw new Error(`runtime chaos invariants failed: ${failed.map((item) => item.id).join(", ")}`)
  }
  return report
}

function normalizeCounts(value) {
  const entries = value instanceof Map ? [...value.entries()] : Object.entries(value)
  return Object.fromEntries(entries
    .map(([operationId, count]) => [operationId, count])
    .sort(([left], [right]) => left.localeCompare(right)))
}

function normalizeState(state) {
  const operationIds = [...(state.operationIds ?? [])]
  return {
    cursor: state.cursor,
    operationIds,
    values: Object.fromEntries(Object.entries(state.values ?? {}).sort(([left], [right]) => left.localeCompare(right))),
  }
}

function monotonicCursorViolations(historyByClient, kernelCursor) {
  const violations = []
  for (const [clientId, history] of Object.entries(historyByClient)) {
    let previous = -1
    for (const [index, cursor] of history.entries()) {
      if (!Number.isSafeInteger(cursor) || cursor < previous || cursor > kernelCursor) {
        violations.push({ clientId, index, previous, cursor, kernelCursor })
      }
      previous = cursor
    }
  }
  return violations
}

function check(id, ok, summary, evidence) {
  return { id, ok, summary, evidence }
}

function canonicalJson(value) {
  return JSON.stringify(value)
}

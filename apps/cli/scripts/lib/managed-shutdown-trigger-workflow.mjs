import {
  exactOperation,
  requireOperationHistory,
  timestamp,
  verifyIdleDeadline,
} from "./managed-shutdown-trigger-observation.mjs"
import { requireValue } from "./managed-shutdown-trigger-config.mjs"

export async function runManagedShutdownScenario({ options, product, context, actionDeadline, askForAction,
  monotonic, remaining, pause, waitForAction }) {
  const { lifecycle, poll, snapshot, waitForExactOperation } = product
  const target = () => context.target

  const finishOneAgent = async () => {
    const before = await snapshot(actionDeadline)
    requireValue(before.summary.runningAgentCount === 0, "agent-start baseline is not idle")
    const previousActivityAt = before.summary.lastActivityChangedAt ?? null
    await askForAction("start_agent_via_normal_path",
      "Use the normal Chariox path to start exactly one agent on this disposable environment.")
    const busy = await poll(({ summary }) => summary.runningAgentCount === 1, actionDeadline)
    const busyAt = timestamp(busy.summary.lastActivityChangedAt, "busy transition time")
    requireValue(previousActivityAt === null || Date.parse(busyAt) > Date.parse(previousActivityAt),
      "agent start did not produce a new activity transition")
    await askForAction("finish_agent_via_normal_provider_path",
      "Finish that agent through its normal provider path and let the turn settle.")
    const idle = await poll(({ summary }) => summary.runningAgentCount === 0
      && typeof summary.lastActivityChangedAt === "string"
      && Date.parse(summary.lastActivityChangedAt) > Date.parse(busyAt), actionDeadline)
    if (idle.summary.autoStopPolicy.idleDelaySeconds === null) {
      requireValue(idle.summary.autoStopDeadlineAt === null && idle.summary.autoStopWarningAt === null,
        "disabled shutdown policy unexpectedly has a deadline")
    } else {
      verifyIdleDeadline(idle.summary)
    }
    return idle
  }

  const awaitAutomaticStop = async (baselineIds, baselineRevision, deadlineAt) => poll(({ summary, operations }) => {
    if (!Array.isArray(operations)) return false
    const matches = operations.filter((operation) => !baselineIds.has(operation.operationId)
      && operation.kind === "stop" && operation.environmentId === target().environmentId
      && operation.desiredRevision > baselineRevision)
    if (matches.length > 1) throw new Error("automatic stop operation is ambiguous")
    if (matches.length !== 1) return false
    const operation = matches[0]
    if (operation.status === "pending" || operation.status === "running") return false
    exactOperation([operation], {
      operationId: operation.operationId,
      kind: "stop",
      desiredRevision: operation.desiredRevision,
    }, target().environmentId)
    requireValue(operation.status === "succeeded" && typeof operation.completedAt === "string"
      && Date.parse(operation.createdAt) >= Date.parse(deadlineAt),
    "automatic stop operation is incomplete or out of order")
    requireValue(summary.desiredState === "stopped" && summary.observedState === "stopped"
      && summary.desiredRevision === operation.desiredRevision
      && summary.observedRevision === operation.desiredRevision,
    "automatic stop state is not reconciled")
    return true
  }, actionDeadline)

  switch (options.descriptor.mode) {
    case "manual": {
      const before = await snapshot(actionDeadline)
      const manualBaseline = new Set(requireOperationHistory(before).map((operation) => operation.operationId))
      await askForAction("manual_stop_via_cloud_ui",
        "Stop this disposable environment through the normal owner-authorized Cloud control UI.")
      const stopped = await poll(({ summary, operations }) => {
        requireValue(Array.isArray(operations), "managed operation history is unavailable")
        const matches = operations.filter((operation) => !manualBaseline.has(operation.operationId)
          && operation.kind === "stop" && operation.environmentId === target().environmentId)
        if (matches.length > 1) throw new Error("manual stop operation is ambiguous")
        if (matches.length !== 1) return false
        const operation = matches[0]
        if (operation.status === "pending" || operation.status === "running") return false
        requireValue(operation.status === "succeeded", "manual stop operation failed")
        requireValue(typeof operation.completedAt === "string"
          && Date.parse(operation.createdAt) >= Date.parse(before.summary.updatedAt)
          && Date.parse(operation.completedAt) >= Date.parse(operation.createdAt)
          && Date.parse(operation.updatedAt) >= Date.parse(operation.completedAt)
          && summary.desiredState === "stopped" && summary.observedState === "stopped"
          && summary.desiredRevision === operation.desiredRevision
          && summary.observedRevision === operation.desiredRevision,
        "manual stop operation is incomplete")
        return true
      }, actionDeadline)
      requireValue(stopped.summary.observedState === "stopped", "manual stop is not observed")
      break
    }
    case "explicit_lifecycle": {
      const stopOperation = await lifecycle("stop", actionDeadline)
      const stopped = await waitForExactOperation(stopOperation, (summary) => summary.observedState === "stopped",
        actionDeadline)
      requireValue(stopped.summary.observedState === "stopped", "explicit stop is not observed")
      break
    }
    case "agents_done":
      await finishOneAgent()
      break
    case "idle_stop": {
      const idle = await finishOneAgent()
      const baselineIds = new Set(requireOperationHistory(idle).map((operation) => operation.operationId))
      await awaitAutomaticStop(baselineIds, idle.summary.desiredRevision, idle.summary.autoStopDeadlineAt)
      break
    }
    case "minimum_runtime": {
      const idle = await finishOneAgent()
      requireValue(idle.summary.autoStopPolicy.minimumRuntimeSeconds === 10_800
        && idle.summary.autoStopPolicy.idleDelaySeconds === 14_400,
      "minimum-runtime policy changed")
      const deadlineAt = verifyIdleDeadline(idle.summary)
      const stopBaseline = new Set(requireOperationHistory(idle).map((operation) => operation.operationId))
      const minimumObservationUntil = monotonic() + 10_800_000
      while (monotonic() < minimumObservationUntil) {
        requireValue(remaining(actionDeadline) > 0, "managed shutdown time bound expired before minimum runtime")
        const current = await snapshot(actionDeadline)
        requireValue(current.summary.desiredState === "running" && current.summary.observedState === "ready"
          && current.summary.runningAgentCount === 0
          && current.summary.desiredRevision === idle.summary.desiredRevision
          && current.summary.lastActivityChangedAt === idle.summary.lastActivityChangedAt
          && current.summary.autoStopDeadlineAt === deadlineAt,
        "managed target left its ready state during the minimum-runtime observation")
        requireValue(!requireOperationHistory(current).some((operation) => !stopBaseline.has(operation.operationId)
          && operation.kind === "stop"), "a stop operation appeared during minimum-runtime observation")
        await waitForAction(pause(Math.min(60_000, minimumObservationUntil - monotonic(), remaining(actionDeadline))))
      }
      const final = await snapshot(actionDeadline, true)
      requireValue(final.summary.desiredState === "running" && final.summary.observedState === "ready"
        && final.summary.runningAgentCount === 0
        && final.summary.desiredRevision === idle.summary.desiredRevision
        && final.summary.lastActivityChangedAt === idle.summary.lastActivityChangedAt
        && final.summary.autoStopDeadlineAt === deadlineAt,
      "managed target did not remain running through the minimum-runtime observation")
      break
    }
    case "disabled": {
      const idle = await finishOneAgent()
      requireValue(idle.summary.autoStopPolicy.idleDelaySeconds === null
        && idle.summary.autoStopDeadlineAt === null, "disabled policy has a shutdown deadline")
      const stopBaseline = new Set(requireOperationHistory(idle).map((operation) => operation.operationId))
      const observeUntil = Math.min(actionDeadline, monotonic() + 120_000)
      await poll(({ summary, operations }) => {
        requireValue(Array.isArray(operations), "managed operation history is unavailable")
        requireValue(summary.autoStopPolicy.idleDelaySeconds === null && summary.autoStopDeadlineAt === null
          && summary.autoStopWarningAt === null && summary.runningAgentCount === 0,
        "disabled shutdown state changed during observation")
        requireValue(!operations.some((operation) => !stopBaseline.has(operation.operationId)
          && operation.kind === "stop"), "stop operation appeared during disabled observation")
        return remaining(observeUntil) <= 0 && summary.observedState === "ready"
      }, observeUntil, 2_000, actionDeadline)
      break
    }
    case "keep_running": {
      const idle = await finishOneAgent()
      await askForAction("keep_running_via_cloud_ui",
        "Use the normal owner-authorized Cloud UI Keep running action before this deadline expires.")
      const kept = await poll(({ summary }) => summary.autoStopDeadlineAt === null
        && summary.runningAgentCount === 0 && summary.desiredRevision === idle.summary.desiredRevision,
      actionDeadline)
      requireValue(kept.summary.lastActivityChangedAt === idle.summary.lastActivityChangedAt,
        "keep-running changed last-agent-finished identity")
      const stopBaseline = new Set(requireOperationHistory(kept).map((operation) => operation.operationId))
      const observeUntil = Math.min(actionDeadline, monotonic() + 120_000)
      await poll(({ summary, operations }) => {
        requireValue(Array.isArray(operations), "managed operation history is unavailable")
        requireValue(!operations.some((operation) => !stopBaseline.has(operation.operationId)
          && operation.kind === "stop"), "a stop operation appeared after Keep running")
        requireValue(summary.autoStopDeadlineAt === null && summary.desiredRevision === idle.summary.desiredRevision
          && summary.lastActivityChangedAt === idle.summary.lastActivityChangedAt,
        "Keep running state changed during observation")
        return remaining(observeUntil) <= 0 && summary.observedState === "ready"
      }, observeUntil, 2_000, actionDeadline)
      break
    }
    case "restart_reconciliation": {
      const idle = await finishOneAgent()
      const baselineIds = new Set(requireOperationHistory(idle).map((operation) => operation.operationId))
      const deadlineAt = verifyIdleDeadline(idle.summary)
      await askForAction("restart_cloud_auto_stop_reconciliation",
        "Use the approved Cloud operations path to restart auto-stop reconciliation for this managed environment.")
      const observeUntil = Math.min(actionDeadline, monotonic() + 120_000)
      await poll(({ summary, operations }) => {
        requireValue(Array.isArray(operations), "managed operation history is unavailable")
        requireValue(!operations.some((operation) => !baselineIds.has(operation.operationId)
          && operation.kind === "stop"), "reconciliation unexpectedly stopped the target")
        requireValue(summary.autoStopDeadlineAt === deadlineAt
          && summary.lastActivityChangedAt === idle.summary.lastActivityChangedAt
          && summary.desiredRevision === idle.summary.desiredRevision
          && summary.observedState === "ready",
        "auto-stop policy was not preserved through reconciliation")
        return remaining(observeUntil) <= 0
      }, observeUntil, 2_000, actionDeadline)
      break
    }
    case "deployment_reconciliation": {
      const idle = await finishOneAgent()
      const baselineIds = new Set(requireOperationHistory(idle).map((operation) => operation.operationId))
      await askForAction("signed_kernel_deployment_reconciliation",
        "Perform the approved signed managed-kernel deployment reconciliation on this target.")
      await awaitAutomaticStop(baselineIds, idle.summary.desiredRevision, idle.summary.autoStopDeadlineAt)
      break
    }
    default:
      throw new Error("shutdown trigger is unsupported")
  }
}

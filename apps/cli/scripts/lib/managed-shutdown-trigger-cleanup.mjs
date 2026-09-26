import { writeCaptureOutput } from "../path1-provider-rebuild-capture.mjs"
import { SHUTDOWN_TRIGGER_LIMITS, requireValue } from "./managed-shutdown-trigger-config.mjs"
import { exactOperation, projectSummary, requireOperationHistory, responseBody } from "./managed-shutdown-trigger-observation.mjs"

export async function cleanupManagedShutdownCapture({ deps, options, context, capture, product, finalDeadline,
  monotonic, remaining, pause, now }) {
  context.cleanupStarted = true

  if (!context.target && context.requestCreateStarted) {
    const discoveryDeadline = Math.min(finalDeadline,
      monotonic() + SHUTDOWN_TRIGGER_LIMITS.cleanupReserveMs)
    while (!context.target && remaining(discoveryDeadline) > 0) {
      try {
        const response = await product.sendUntil(deps.requests.listManagedEnvironmentCatalogRequest(), discoveryDeadline)
        const catalog = responseBody(response, "ManagedEnvironmentCatalog").catalog
        const candidates = (catalog?.environments ?? []).map(projectSummary)
          .filter((summary) => summary.name === context.targetName
            && summary.computeClass === options.computeClass && summary.region === options.region)
        if (candidates.length === 1) {
          context.target = candidates[0]
          capture.target = context.target
        } else if (candidates.length > 1) {
          context.cleanupFailed = true
          break
        }
      } catch {
        // Retry the exact generated-name lookup within the cleanup reserve.
      }
      if (!context.target && !context.cleanupFailed) {
        await pause(Math.min(SHUTDOWN_TRIGGER_LIMITS.pollMs, Math.max(0, remaining(discoveryDeadline))))
      }
    }
    if (!context.target) context.cleanupFailed = true
  }

  if (context.target) {
    try {
      const current = await product.snapshot(finalDeadline)
      if (current.summary.observedState === "deleted") {
        const deletions = requireOperationHistory(current).filter((operation) => operation.kind === "delete"
          && operation.desiredRevision === current.summary.desiredRevision
          && operation.status === "succeeded" && typeof operation.completedAt === "string")
        requireValue(deletions.length === 1, "existing deletion receipt is unavailable or ambiguous")
        exactOperation([deletions[0]], {
          operationId: deletions[0].operationId,
          kind: "delete",
          desiredRevision: current.summary.desiredRevision,
        }, context.target.environmentId)
      } else {
        const deleteOperation = await product.lifecycle("delete", finalDeadline)
        const deleted = await product.waitForExactOperation(deleteOperation,
          (summary) => summary.desiredState === "deleted" && summary.observedState === "deleted",
          finalDeadline)
        requireValue(deleted.summary.observedState === "deleted", "target deletion is not observed")
      }
    } catch {
      context.cleanupFailed = true
    }
  }

  capture.finishedAt = now().toISOString()
  try {
    capture.outputPath = await (deps.writeEvidence ?? writeCaptureOutput)(options.output, capture)
  } catch {
    context.outputFailed = true
  }
  await deps.client.close().catch(() => {})
}

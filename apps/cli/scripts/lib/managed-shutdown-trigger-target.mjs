import { nonEmpty, requireValue } from "./managed-shutdown-trigger-config.mjs"
import { projectOperation, projectSummary, recordOperation, requireOperationHistory, responseBody } from "./managed-shutdown-trigger-observation.mjs"

export async function createManagedShutdownTarget({ deps, options, runId, context, capture, product, actionDeadline }) {
  const catalogResponse = await product.sendUntil(
    deps.requests.listManagedEnvironmentCatalogRequest(), actionDeadline)
  const catalog = responseBody(catalogResponse, "ManagedEnvironmentCatalog").catalog
  requireValue(Array.isArray(catalog?.computeClasses)
    && catalog.computeClasses.some((item) => item.computeClass === options.computeClass
      && Array.isArray(item.regions) && item.regions.includes(options.region)),
  "bounded compute class or region is not available")

  context.requestCreateStarted = true
  const createResponse = await product.sendUntil(deps.requests.createManagedEnvironmentRequest({
    clientRequestId: runId,
    name: context.targetName,
    region: options.region,
    computeClass: options.computeClass,
    autoStopPolicy: options.descriptor.policy,
    contextPlan: {
      sourceTargetId: null,
      kernelContext: "empty",
      developmentSetup: { kind: "empty" },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
  }), actionDeadline, true)
  const created = responseBody(createResponse, "ManagedEnvironmentCreated").result
  const createdSummary = projectSummary(created.environment)
  requireValue(createdSummary.name === context.targetName && createdSummary.computeClass === options.computeClass
    && createdSummary.region === options.region
    && JSON.stringify(createdSummary.autoStopPolicy) === JSON.stringify(options.descriptor.policy),
  "created managed target does not match this task")
  context.target = createdSummary
  capture.target = createdSummary
  const createOperation = projectOperation(created.operation)
  requireValue(createOperation.environmentId === createdSummary.environmentId
    && createOperation.requestedByUserId === createdSummary.createdByUserId && createOperation.kind === "create"
    && createOperation.desiredRevision === createdSummary.desiredRevision,
  "managed create operation identity is incomplete")
  recordOperation(capture, createOperation)
  const ready = await product.waitForExactOperation(createOperation,
    (summary) => summary.observedState === "ready", actionDeadline)
  requireValue(ready.summary.autoStopPolicy.minimumRuntimeSeconds === options.descriptor.policy.minimumRuntimeSeconds
    && ready.summary.autoStopPolicy.idleDelaySeconds === options.descriptor.policy.idleDelaySeconds,
  "persisted managed shutdown policy changed")
  context.target = {
    ...context.target,
    runtimeMachineId: nonEmpty(ready.summary.runtimeMachineId, "ready runtime machine identity"),
    runtimeKernelId: nonEmpty(ready.summary.runtimeKernelId, "ready runtime kernel identity"),
  }
  capture.target = context.target
  requireOperationHistory(ready)
}

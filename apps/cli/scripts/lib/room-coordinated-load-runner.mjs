import { COORDINATED_LOAD_UNRUN_GATES } from "./room-coordinated-load-plan.mjs"

export const COORDINATED_LOAD_VIEWERS = Object.freeze([
  Object.freeze({ id: "web-local", route: "local" }),
  Object.freeze({ id: "web-relay", route: "relay" }),
])
export const COORDINATED_LOAD_TUIS = Object.freeze([
  Object.freeze({ id: "local-tui", route: "local" }),
  Object.freeze({ id: "relay-tui", route: "relay" }),
])

export async function runRoomCoordinatedLoad(plan, runtime, { signal = null } = {}) {
  requireRuntime(runtime)
  const ownedTasks = []
  const samples = []
  const workflowStatuses = []
  let stage = "inventory_before"
  let before = null
  let after = null
  let slowViewer = null
  let failureCode = null
  let failedStage = null
  let workflowTask = null
  let measurementStartedAt = null

  try {
    before = await runtime.captureInventory(plan, { phase: "before", ownedTasks })
    stage = "verify_prepared_rooms_and_slices"
    assertPreparedIdentities(plan, await runtime.verifyPrepared(plan))

    stage = "start_web_viewers"
    for (const viewer of COORDINATED_LOAD_VIEWERS) {
      ownedTasks.push(assertOwnedTask(
        await runtime.startViewer(plan, viewer), plan.runId, "viewer", viewer.id,
      ))
    }

    stage = "start_tui_clients"
    for (const tui of COORDINATED_LOAD_TUIS) {
      ownedTasks.push(assertOwnedTask(
        await runtime.startTui(plan, tui), plan.runId, "tui", tui.id,
      ))
    }

    stage = "invoke_workflow"
    workflowTask = assertOwnedTask(
      await runtime.startWorkflow(plan), plan.runId, "workflow", plan.workflow.workflowId,
    )
    ownedTasks.push(workflowTask)
    measurementStartedAt = runtime.now()

    const sampleCount = Math.ceil(plan.limits.durationMs / plan.limits.sampleIntervalMs)
    for (let index = 0; index < sampleCount; index += 1) {
      if (signal?.aborted) throw Object.assign(new Error("interrupted"), { stage: "interrupted" })
      if (index > 0 && runtime.now() - measurementStartedAt >= plan.limits.durationMs) break

      stage = "sample_clients_and_resources"
      const sample = validateSample(await runtime.sample(plan, {
        sampleIndex: index,
        ownedTasks,
        workflowTask,
        elapsedMs: Math.max(0, runtime.now() - measurementStartedAt),
      }), plan, index)
      samples.push(sample)
      if (sample.workflowStatus) workflowStatuses.push(sample.workflowStatus)
      if (sample.localKernelLatencyMs > plan.limits.maxKernelLatencyMs
        || sample.relayKernelLatencyMs > plan.limits.maxKernelLatencyMs
        || sample.aggregateContainerMemoryBytes > plan.limits.maxMemoryBytes
        || sample.aggregateContainerCpuPercent > plan.limits.maxCpuPercent) {
        failureCode = "configured_resource_or_latency_bound_exceeded"
        failedStage = stage
        break
      }
      if (sample.workflowStatus === "failed" || sample.workflowStatus === "stopped") {
        failureCode = "prepared_workflow_failed"
        failedStage = stage
        break
      }
      if (index === plan.slowViewer.afterSample) {
        stage = "slow_viewer_injection"
        const task = ownedTasks.find((entry) => entry.kind === "viewer" && entry.id === plan.slowViewer.viewerId)
        slowViewer = validateSlowViewerObservation(
          await runtime.injectSlowViewer(task, plan.slowViewer.delayMs),
          plan.slowViewer,
        )
      }
      {
        stage = "sample_interval"
        const nextSampleAtMs = Math.min((index + 1) * plan.limits.sampleIntervalMs, plan.limits.durationMs)
        const sleepMs = Math.max(0, nextSampleAtMs - (runtime.now() - measurementStartedAt))
        if (sleepMs > 0) await runtime.sleep(sleepMs, signal)
      }
    }
    if (!slowViewer) {
      failureCode = failureCode ?? "slow_viewer_injection_missing"
      failedStage = failedStage ?? "slow_viewer_injection"
    }
    if (!workflowStatuses.some((status) => ["running", "completing", "completed"].includes(status))) {
      failureCode = failureCode ?? "workflow_execution_not_observed"
      failedStage = failedStage ?? "sample_clients_and_resources"
    }
  } catch (error) {
    failureCode = failureCode ?? (signal?.aborted ? "interrupted" : "coordinated_load_step_failed")
    failedStage = failedStage ?? error?.stage ?? stage
  } finally {
    stage = "cleanup_owned_tasks"
    for (const task of [...ownedTasks].reverse()) {
      try {
        const stopped = await runtime.stopOwnedTask(task)
        if (!stopped || stopped.ownerRunId !== plan.runId || stopped.stopped !== true || stopped.remaining === true) {
          failureCode = failureCode ?? "owned_task_cleanup_incomplete"
          failedStage = failedStage ?? stage
        }
      } catch {
        failureCode = failureCode ?? "owned_task_cleanup_incomplete"
        failedStage = failedStage ?? stage
      }
    }
    stage = "inventory_after"
    try {
      after = await runtime.captureInventory(plan, { phase: "after", ownedTasks })
    } catch {
      failureCode = failureCode ?? "post_cleanup_inventory_unavailable"
      failedStage = failedStage ?? stage
    }
  }

  const inventory = before && after ? compareInventories(before, after, plan.runId) : null
  if (inventory && !inventory.clean) {
    failureCode = failureCode ?? "owned_resource_inventory_changed"
    failedStage = failedStage ?? "inventory_after"
  }
  return {
    schema: "chariox.room_coordinated_load.report.v1",
    runId: plan.runId,
    status: failureCode ? "failed" : "measured",
    acceptance: "not_proven",
    failedStage: failureCode ? failedStage : null,
    failureCode,
    timing: {
      requestedDurationMs: plan.limits.durationMs,
      sampleCount: samples.length,
      observedElapsedMs: measurementStartedAt === null
        ? 0
        : Math.max(0, runtime.now() - measurementStartedAt),
    },
    approvedSliceCount: plan.approval.approvedMaxHeadedSlices,
    verifiedPreparedSliceCount: plan.headedSlices.length,
    approvalReference: plan.approval.reference,
    samples,
    slowViewer,
    workflow: {
      workflowId: workflowTask?.id ?? null,
      workflowRunId: workflowTask?.workflowRunId ?? null,
      observedStatuses: [...new Set(workflowStatuses)],
      acceptedByKernel: Boolean(workflowTask),
      executionObserved: workflowStatuses.some((status) => ["running", "completing", "completed"].includes(status)),
    },
    cleanup: inventory,
    unrunGates: COORDINATED_LOAD_UNRUN_GATES,
  }
}

export function assertPreparedIdentities(plan, snapshot) {
  if (!snapshot || !Array.isArray(snapshot.slices)
    || snapshot.slices.length !== plan.headedSlices.length
    || snapshot.approvedMaxHeadedSlices !== plan.approval.approvedMaxHeadedSlices) {
    throw new Error("kernel did not verify the exact approved prepared-slice set")
  }
  const actualById = new Map(snapshot.slices.map((slice) => [slice.sliceId, slice]))
  for (const expected of plan.headedSlices) {
    const actual = actualById.get(expected.sliceId)
    if (!actual || actual.sliceName !== expected.sliceName || actual.roomId !== expected.roomId
      || actual.environmentId !== expected.environmentId
      || actual.runtimeGeneration !== expected.runtimeGeneration
      || actual.displayMode !== "headed" || actual.sliceStatus !== "running"
      || actual.environmentLifecycle !== "ready"
      || actual.workerKernelId !== expected.workerKernelId
      || actual.workerMachineId !== expected.workerMachineId) {
      throw new Error("kernel prepared-slice identity or readiness differs from the exact config")
    }
  }
  return true
}

export function compareInventories(before, after, runId) {
  const beforeOwned = canonicalInventory(before)
  const afterOwned = canonicalInventory(after)
  return {
    clean: stableJson(beforeOwned) === stableJson(afterOwned)
      && afterOwned.remainingOwnedPids.length === 0,
    before: beforeOwned,
    after: afterOwned,
    changed: stableJson(beforeOwned) !== stableJson(afterOwned),
    remainingOwnedPids: afterOwned.remainingOwnedPids,
    ownerRunId: runId,
  }
}

function canonicalInventory(value) {
  if (!value || typeof value !== "object") throw new Error("inventory is missing")
  const array = (name) => {
    if (!Array.isArray(value[name])) throw new Error(`inventory ${name} is malformed`)
    return [...value[name]].map(String).sort()
  }
  return {
    processes: array("ownedProcessIds"),
    containers: array("containers"),
    volumes: array("volumes"),
    ports: array("ports"),
    remainingOwnedPids: array("remainingOwnedPids"),
  }
}

function assertOwnedTask(task, runId, kind, id) {
  if (!task || task.ownerRunId !== runId || task.kind !== kind || task.id !== id
    || task.started !== true || typeof task.stopToken !== "string" || !task.stopToken) {
    throw new Error(`runtime did not return a task-owned ${kind} handle`)
  }
  return task
}

function validateSlowViewerObservation(observation, expected) {
  if (!observation || observation.viewerId !== expected.viewerId
    || observation.requestedDelayMs !== expected.delayMs
    || observation.observedDelayMs < expected.delayMs
    || observation.observedDelayMs > expected.delayMs + 250) {
    throw new Error("slow viewer was not deliberately delayed and observed")
  }
  return {
    viewerId: expected.viewerId,
    requestedDelayMs: expected.delayMs,
    observedDelayMs: observation.observedDelayMs,
    injectedOnce: true,
  }
}

function validateSample(sample, plan, index) {
  if (!sample || typeof sample !== "object" || sample.sampleIndex !== index
    || typeof sample.capturedAt !== "string" || !Number.isFinite(Date.parse(sample.capturedAt))) {
    throw new Error("live sample identity or timestamp is invalid")
  }
  const numericFields = [
    "elapsedMs", "localKernelLatencyMs", "relayKernelLatencyMs", "aggregateContainerMemoryBytes",
    "aggregateContainerCpuPercent", "viewerLocalFrameBytes", "viewerRelayFrameBytes",
    "hostFreeMemoryBytes", "processRssBytes", "ownedProcessCount", "openListenerCount",
  ]
  for (const field of numericFields) {
    if (!Number.isFinite(sample[field]) || sample[field] < 0) throw new Error(`live sample ${field} is invalid`)
  }
  if (!new Set(["created", "running", "waiting", "completing", "paused", "completed", "failed", "stopped", "unknown"])
    .has(sample.workflowStatus)) {
    throw new Error("live sample workflow status is invalid")
  }
  if (sample.viewerLocalFrameBytes <= 0 || sample.viewerRelayFrameBytes <= 0) {
    throw new Error("both live viewer streams must deliver actual frame bytes")
  }
  if (sample.ownedProcessCount < 2) throw new Error("both normal TUI clients must remain owned and active")
  return Object.fromEntries([
    "sampleIndex", "capturedAt", ...numericFields, "workflowStatus",
  ].map((field) => [field, sample[field]]))
}

function requireRuntime(runtime) {
  const methods = [
    "now", "sleep", "captureInventory", "verifyPrepared", "startViewer", "startTui",
    "startWorkflow", "injectSlowViewer", "sample", "stopOwnedTask",
  ]
  if (!runtime || methods.some((method) => typeof runtime[method] !== "function")) {
    throw new Error("coordinated load runtime is incomplete")
  }
}

function stableJson(value) {
  return JSON.stringify(value)
}

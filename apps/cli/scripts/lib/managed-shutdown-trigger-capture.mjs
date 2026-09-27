import { randomUUID } from "node:crypto"
import { createInterface } from "node:readline/promises"
import { stdin, stderr } from "node:process"

import { validateCaptureOutput } from "../path1-provider-rebuild-capture.mjs"
import {
  requireValue,
  SHUTDOWN_TRIGGER_LIMITS,
  SHUTDOWN_TRIGGER_SCHEMA,
  USER_ACTIONS,
} from "./managed-shutdown-trigger-config.mjs"
import { createManagedShutdownProduct } from "./managed-shutdown-trigger-product.mjs"
import { createManagedShutdownTarget } from "./managed-shutdown-trigger-target.mjs"
import { runManagedShutdownScenario } from "./managed-shutdown-trigger-workflow.mjs"
import { cleanupManagedShutdownCapture } from "./managed-shutdown-trigger-cleanup.mjs"

function abortable(promise, signal) {
  if (!signal) return promise
  if (signal.aborted) {
    void Promise.resolve(promise).catch(() => {})
    return Promise.reject(new Error("capture interrupted"))
  }
  return new Promise((resolvePromise, rejectPromise) => {
    const cleanup = () => signal.removeEventListener("abort", onAbort)
    const onAbort = () => {
      cleanup()
      rejectPromise(new Error("capture interrupted"))
    }
    signal.addEventListener("abort", onAbort, { once: true })
    Promise.resolve(promise).then(
      (value) => { cleanup(); resolvePromise(value) },
      (error) => { cleanup(); rejectPromise(error) },
    )
  })
}

function promptWithinDeadline(prompt, message, deadline, { signal, remaining, setTimer, clearTimer }) {
  const initialRemaining = remaining(deadline)
  requireValue(initialRemaining > 0, "managed shutdown time bound expired before owner action")
  const actionController = new AbortController()

  return new Promise((resolvePromise, rejectPromise) => {
    let timer
    let settled = false
    const cleanup = () => {
      if (timer !== undefined) clearTimer(timer)
      signal?.removeEventListener("abort", onAbort)
    }
    const finish = (settle, value) => {
      if (settled) return false
      settled = true
      cleanup()
      settle(value)
      return true
    }
    const onAbort = () => {
      const reason = signal?.reason ?? new Error("capture interrupted")
      if (!actionController.signal.aborted) actionController.abort(reason)
      finish(rejectPromise, new Error("capture interrupted"))
    }
    const onDeadline = () => {
      if (settled) return
      const milliseconds = remaining(deadline)
      if (milliseconds > 0) {
        timer = setTimer(onDeadline, milliseconds)
        timer?.unref?.()
        return
      }
      const error = new Error("managed shutdown time bound expired while awaiting owner action")
      if (!finish(rejectPromise, error)) return
      actionController.abort(error)
    }

    if (signal?.aborted) {
      actionController.abort(signal.reason)
      rejectPromise(new Error("capture interrupted"))
      return
    }
    signal?.addEventListener("abort", onAbort, { once: true })
    timer = setTimer(onDeadline, initialRemaining)
    timer?.unref?.()

    let action
    try {
      action = prompt(message, actionController.signal)
    } catch (error) {
      actionController.abort(error)
      finish(rejectPromise, error)
      return
    }
    Promise.resolve(action).then(
      (value) => finish(resolvePromise, value),
      (error) => finish(rejectPromise, error),
    )
  })
}

export async function runManagedShutdownTrigger(options, deps) {
  await validateCaptureOutput(options.output)
  const now = deps.now ?? (() => new Date())
  const monotonic = deps.monotonic ?? (() => performance.now())
  const pause = deps.pause ?? ((milliseconds) => new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds)))
  const setActionTimeout = deps.setTimeout ?? setTimeout
  const clearActionTimeout = deps.clearTimeout ?? clearTimeout
  const prompt = deps.ask ?? (async (message, signal) => {
    const promptInterface = createInterface({ input: stdin, output: stderr })
    try {
      await abortable(promptInterface.question(`${message}\nPress Enter after the requested product action is complete. `), signal)
    }
    finally { promptInterface.close() }
  })
  const capture = {
    schema: SHUTDOWN_TRIGGER_SCHEMA,
    scenario: options.scenario,
    observationSurface: "owner-authorized-local-kernel-cloud-projection",
    startedAt: now().toISOString(),
    resourceBound: {
      maximumConcurrentTargets: 1,
      computeClass: options.computeClass,
      maximumBillableSeconds: options.maxBillableSeconds,
    },
    requestedAutoStopPolicy: options.descriptor.policy,
    target: null,
    observations: [],
    operations: [],
    requiredUserActions: [],
  }
  const runId = (deps.id ?? randomUUID)()
  const context = {
    target: null,
    targetName: `mp09-${options.scenario}-${runId}`,
    requestCreateStarted: false,
    cleanupStarted: false,
    workflowFailed: false,
    cleanupFailed: false,
    outputFailed: false,
  }
  const started = monotonic()
  const finalDeadline = started + options.maxBillableSeconds * 1_000
  const actionDeadline = finalDeadline - SHUTDOWN_TRIGGER_LIMITS.cleanupReserveMs
  const remaining = (deadline) => Math.floor(deadline - monotonic())
  const waitForAction = (promise) => abortable(promise, context.cleanupStarted ? undefined : deps.signal)
  const askForAction = async (action, message) => {
    requireValue(USER_ACTIONS.has(action), "required user action is unsupported")
    capture.requiredUserActions.push({ action, requestedAt: now().toISOString() })
    await promptWithinDeadline(prompt, message, actionDeadline, {
      signal: deps.signal,
      remaining,
      setTimer: setActionTimeout,
      clearTimer: clearActionTimeout,
    })
  }
  const product = createManagedShutdownProduct({
    deps,
    options,
    capture,
    getTarget: () => context.target,
    isCleanupStarted: () => context.cleanupStarted,
    waitForAction,
    remaining,
    now,
    pause,
    runId,
  })

  try {
    await createManagedShutdownTarget({ deps, options, runId, context, capture, product, actionDeadline })

    await runManagedShutdownScenario({
      options,
      product,
      context,
      actionDeadline,
      askForAction,
      monotonic,
      remaining,
      pause,
      waitForAction,
    })
  } catch {
    context.workflowFailed = true
  }

  await cleanupManagedShutdownCapture({
    deps,
    options,
    context,
    capture,
    product,
    finalDeadline,
    monotonic,
    remaining,
    pause,
    now,
  })
  if (context.workflowFailed || context.cleanupFailed || context.outputFailed) {
    throw new Error("managed shutdown observation is incomplete; no acceptance verdict")
  }
  return capture
}

import assert from "node:assert/strict"
import { getEventListeners } from "node:events"
import { mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { parseArguments, runManagedShutdownTrigger, SHUTDOWN_SCENARIOS } from "./live-managed-shutdown-trigger-drill.mjs"
import {
  MANAGED_SHUTDOWN_WARNING_SECONDS,
  SHUTDOWN_TRIGGER_LIMITS,
} from "./lib/managed-shutdown-trigger-config.mjs"
import { projectSummary, verifyIdleDeadline } from "./lib/managed-shutdown-trigger-observation.mjs"

const CONFIRMATION = "CREATE-AND-DELETE-ONE-MANAGED-TARGET"
const baseTime = "2026-09-26T05:00:00.000Z"

function argumentsFor(output, scenario = "shutdown_explicit_lifecycle_reconciliation",
  maxBillableSeconds = SHUTDOWN_SCENARIOS[scenario].minimumSeconds) {
  return [
    "--scenario", scenario,
    "--kernel-url", "ws://127.0.0.1:4488/kernel",
    "--region", "hel1",
    "--compute-class", "agent-small",
    "--max-billable-seconds", String(maxBillableSeconds),
    "--confirm-one-target", CONFIRMATION,
    "--output", output,
  ]
}

function environment({ state = "ready", revision = 1, policy = { minimumRuntimeSeconds: 0, idleDelaySeconds: null } } = {}) {
  return {
    environmentId: "environment-fixture-1",
    accountId: "account-fixture-1",
    createdByUserId: "user-fixture-1",
    name: "mp09-shutdown_explicit_lifecycle_reconciliation-run-fixture",
    region: "hel1",
    computeClass: "agent-small",
    desiredState: state === "ready" ? "running" : state,
    observedState: state,
    desiredRevision: revision,
    observedRevision: revision,
    autoStopPolicy: policy,
    runtimeMachineId: "machine-fixture-1",
    runtimeKernelId: "kernel-fixture-1",
    runningAgentCount: 0,
    lastActivityReportedAt: null,
    lastActivityChangedAt: null,
    autoStopWarningAt: null,
    autoStopDeadlineAt: null,
    runtimeStartedAt: baseTime,
    createdAt: baseTime,
    updatedAt: baseTime,
  }
}

function operation(id, kind, revision, status = "succeeded", createdAt = baseTime) {
  return {
    operationId: id,
    environmentId: "environment-fixture-1",
    requestedByUserId: "user-fixture-1",
    kind,
    desiredRevision: revision,
    status,
    attempt: 1,
    completedAt: status === "succeeded" ? createdAt : null,
    createdAt,
    updatedAt: createdAt,
  }
}

function fakeProductPath({ includeHistory = true, policy } = {}) {
  let current = environment({ policy })
  const operations = []
  const calls = []
  const advanceAutomaticStop = (observedAt) => {
    if (current.observedState !== "ready" || current.desiredState !== "running"
      || current.autoStopDeadlineAt === null || Date.parse(observedAt) < Date.parse(current.autoStopDeadlineAt)) {
      return false
    }
    const deadlineAt = current.autoStopDeadlineAt
    const revision = current.desiredRevision + 1
    current = {
      ...current,
      desiredState: "stopped",
      observedState: "stopped",
      desiredRevision: revision,
      observedRevision: revision,
      autoStopWarningAt: null,
      autoStopDeadlineAt: null,
      updatedAt: deadlineAt,
    }
    operations.push(operation(`auto-stop-op-${revision}`, "stop", revision, "succeeded", deadlineAt))
    return true
  }
  const send = async (request) => {
    const [name] = Object.keys(request)
    calls.push([name, request[name]])
    if (name === "ListManagedEnvironmentCatalog") {
      return { ManagedEnvironmentCatalog: { catalog: {
        computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
        environments: [],
      } } }
    }
    if (name === "CreateManagedEnvironment") {
      current = { ...environment({ policy: request[name].autoStopPolicy }), name: request[name].name }
      const created = operation("create-op-1", "create", 1)
      operations.push(created)
      return { ManagedEnvironmentCreated: { result: { environment: current, operation: created } } }
    }
    if (name === "GetManagedEnvironment") {
      assert.equal(request[name].environmentId, current.environmentId)
      return { ManagedEnvironment: {
        environment: current,
        ...(includeHistory ? { operations: [...operations] } : {}),
      } }
    }
    if (name === "RequestManagedEnvironmentLifecycle") {
      const { action, environmentId } = request[name]
      assert.equal(environmentId, current.environmentId)
      const revision = current.desiredRevision + 1
      const state = action === "delete" ? "deleted" : "stopped"
      const createdAt = current.updatedAt
      current = { ...environment({ state, revision, policy: current.autoStopPolicy }), name: current.name }
      const requested = operation(`${action}-op-${revision}`, action, revision, "pending", createdAt)
      operations.push({ ...requested, status: "succeeded", completedAt: createdAt })
      return { ManagedEnvironmentLifecycleRequested: { result: { environment: current, operation: requested } } }
    }
    throw new Error(`unexpected request ${name}`)
  }
  const requests = {
    listManagedEnvironmentCatalogRequest: () => ({ ListManagedEnvironmentCatalog: null }),
    createManagedEnvironmentRequest: (input) => ({ CreateManagedEnvironment: input }),
    getManagedEnvironmentRequest: (environmentId) => ({ GetManagedEnvironment: { environmentId } }),
    requestManagedEnvironmentLifecycleRequest: (input) => ({ RequestManagedEnvironmentLifecycle: input }),
  }
  return {
    calls,
    send,
    requests,
    operations,
    advanceAutomaticStop,
    updateCurrent(update) { current = { ...current, ...update } },
  }
}

function atSecond(second) {
  return new Date(Date.parse(baseTime) + second * 1_000).toISOString()
}

async function scratch(t) {
  const parent = await realpath(tmpdir())
  const root = await mkdtemp(join(parent, "chariox-mp09-driver-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  return root
}

test("CLI accepts only the twelve bounded scenarios and no operator-supplied result flags", () => {
  assert.deepEqual(Object.keys(SHUTDOWN_SCENARIOS), [
    "shutdown_agents_done", "shutdown_idle_15m", "shutdown_idle_30m", "shutdown_minimum_3h",
    "shutdown_disabled", "shutdown_keep_running", "shutdown_restart_reconciliation",
    "shutdown_all_clients_disconnected", "shutdown_manual",
    "shutdown_custom", "shutdown_explicit_lifecycle_reconciliation", "shutdown_deployment_reconciliation",
  ])
  const valid = argumentsFor("/tmp/evidence.json")
  assert.equal(parseArguments(valid).scenario, "shutdown_explicit_lifecycle_reconciliation")
  for (const unsupported of ["--status", "--evidence", "--operation-id", "--passed"]) {
    assert.throws(() => parseArguments([...valid, unsupported, "claimed"]), /invalid arguments/)
  }
  assert.throws(() => parseArguments(argumentsFor("relative.json")), /absolute/)
  const overLimit = argumentsFor("/tmp/evidence.json")
  overLimit[overLimit.indexOf("--max-billable-seconds") + 1] = "14401"
  assert.throws(() => parseArguments(overLimit), /time bound/)
  const belowMinimum = argumentsFor("/tmp/evidence.json", "shutdown_minimum_3h")
  belowMinimum[belowMinimum.indexOf("--max-billable-seconds") + 1] = "12599"
  assert.throws(() => parseArguments(belowMinimum), /time bound/)
})

test("idle deadline validation matches Cloud's exact 30-second warning and three-way deadline maximum", () => {
  const policy = { minimumRuntimeSeconds: 10_800, idleDelaySeconds: 900 }
  const summary = projectSummary({
    ...environment({ policy }),
    runtimeStartedAt: baseTime,
    lastActivityChangedAt: atSecond(2),
    lastActivityReportedAt: atSecond(3),
    autoStopDeadlineAt: atSecond(10_800),
    autoStopWarningAt: atSecond(10_800 - MANAGED_SHUTDOWN_WARNING_SECONDS),
  })
  assert.equal(verifyIdleDeadline(summary), atSecond(10_800))
  assert.throws(() => verifyIdleDeadline({
    ...summary,
    autoStopWarningAt: atSecond(10_800 - 300),
  }), /30-second warning/)
  assert.throws(() => verifyIdleDeadline({
    ...summary,
    autoStopDeadlineAt: atSecond(10_801),
    autoStopWarningAt: atSecond(10_801 - MANAGED_SHUTDOWN_WARNING_SECONDS),
  }), /Cloud deadline/)
})

test("explicit lifecycle uses owner IPC stop and delete receipts and writes private external raw observations", async (t) => {
  const root = await scratch(t)
  const output = join(root, "observation.json")
  const product = fakeProductPath()
  let captured
  const options = parseArguments(argumentsFor(output))
  const result = await runManagedShutdownTrigger(options, {
    client: { send: product.send, close: async () => {} },
    requests: product.requests,
    send: product.send,
    pause: async () => {},
    id: () => "run-fixture-1",
    writeEvidence: async (path, value) => {
      captured = structuredClone(value)
      const { writeCaptureOutput } = await import("./path1-provider-rebuild-capture.mjs")
      return writeCaptureOutput(path, value)
    },
  })
  assert.equal(result.target.environmentId, "environment-fixture-1")
  assert.deepEqual(product.calls.map(([name]) => name), [
    "ListManagedEnvironmentCatalog", "CreateManagedEnvironment", "GetManagedEnvironment",
    "GetManagedEnvironment", "RequestManagedEnvironmentLifecycle", "GetManagedEnvironment",
    "GetManagedEnvironment", "GetManagedEnvironment", "RequestManagedEnvironmentLifecycle",
    "GetManagedEnvironment",
  ])
  const actions = product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
    .map(([, body]) => [body.environmentId, body.action])
  assert.deepEqual(actions, [["environment-fixture-1", "stop"], ["environment-fixture-1", "delete"]])
  const createInput = product.calls.find(([name]) => name === "CreateManagedEnvironment")[1]
  assert.equal(createInput.contextPlan.providerAccounts.kind, "none")
  assert.equal(createInput.contextPlan.gitCredentials.kind, "none")
  assert.deepEqual(result.operations.map(({ operationId, kind, desiredRevision, status, completedAt }) =>
    [operationId, kind, desiredRevision, status, completedAt]), [
    ["create-op-1", "create", 1, "succeeded", baseTime],
    ["stop-op-2", "stop", 2, "succeeded", baseTime],
    ["delete-op-3", "delete", 3, "succeeded", baseTime],
  ])
  assert.equal("verdict" in captured, false)
  assert.equal("passed" in captured, false)
  assert.equal("operatorResult" in captured, false)
  assert.equal("lastErrorMessage" in captured.target, false)
  assert.equal("failureMessage" in captured.operations[0], false)
  assert.equal(captured.observationSurface, "owner-authorized-local-kernel-cloud-projection")
  const written = JSON.parse(await readFile(output, "utf8"))
  assert.deepEqual(written, captured)
  assert.equal((await stat(output)).mode & 0o777, 0o600)
})

test("existing and symlink output paths are rejected before any owner IPC request", async (t) => {
  const root = await scratch(t)
  const existing = join(root, "existing.json")
  await writeFile(existing, "retained")
  const product = fakeProductPath()
  const deps = { client: { send: product.send, close: async () => {} }, requests: product.requests, send: product.send }
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(existing)), deps), /output/)
  assert.equal(product.calls.length, 0)
  assert.equal(await readFile(existing, "utf8"), "retained")

  const target = join(root, "target.json")
  const alias = join(root, "alias.json")
  await writeFile(target, "retained")
  await symlink(target, alias)
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(alias)), deps), /output/)
  assert.equal(product.calls.length, 0)
  assert.equal(await readFile(target, "utf8"), "retained")
})

test("missing owner operation history cannot become a completed observation", async (t) => {
  const root = await scratch(t)
  const output = join(root, "incomplete.json")
  const product = fakeProductPath({ includeHistory: false })
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(output)), {
    client: { send: product.send, close: async () => {} },
    requests: product.requests,
    send: product.send,
    pause: async () => {},
    id: () => "run-no-history",
  }), /no acceptance verdict/)
  const actions = product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
    .map(([, body]) => body.action)
  assert.deepEqual(actions, ["delete"])
  const incomplete = JSON.parse(await readFile(output, "utf8"))
  assert.equal("verdict" in incomplete, false)
  assert.equal("passed" in incomplete, false)
  assert.equal(incomplete.operations.some((item) => item.kind === "stop"), false)
  assert.equal(incomplete.operations.find((item) => item.kind === "delete")?.status, "pending")
})

test("a never-settling owner prompt expires at the action deadline and leaves time for exact delete cleanup", async (t) => {
  const root = await scratch(t)
  const output = join(root, "prompt-timeout.json")
  const options = parseArguments(argumentsFor(output, "shutdown_agents_done"))
  const actionBoundMs = options.maxBillableSeconds * 1_000 - SHUTDOWN_TRIGGER_LIMITS.cleanupReserveMs
  const product = fakeProductPath({ policy: options.descriptor.policy })
  const controller = new AbortController()
  const signal = controller.signal
  let runSignalAbortEvents = 0
  const recordRunSignalAbort = () => { runSignalAbortEvents += 1 }
  signal.addEventListener("abort", recordRunSignalAbort)
  let monotonicNow = 0
  let promptStarted
  const ownerActionStarted = new Promise((resolve) => { promptStarted = resolve })
  let promptClosed = false
  let promptSignal
  let clientClosed = false
  let timerDelay
  let timerCallback
  let timerCleared = false
  const run = runManagedShutdownTrigger(options, {
    client: { send: product.send, close: async () => { clientClosed = true } },
    requests: product.requests,
    send: product.send,
    id: () => "run-prompt-timeout",
    now: () => new Date(Date.parse(baseTime) + monotonicNow),
    monotonic: () => monotonicNow,
    pause: async () => {},
    setTimeout: (callback, milliseconds) => {
      timerCallback = callback
      timerDelay = milliseconds
      return "action-deadline"
    },
    clearTimeout: (handle) => {
      assert.equal(handle, "action-deadline")
      timerCleared = true
    },
    signal,
    ask: (message, suppliedPromptSignal) => {
      assert.match(message, /start exactly one agent/)
      promptSignal = suppliedPromptSignal
      promptStarted()
      let onAbort
      const actionPrompt = new Promise((_, reject) => {
        onAbort = () => reject(new Error("prompt closed"))
        promptSignal.addEventListener("abort", onAbort, { once: true })
      })
      return actionPrompt.finally(() => {
        promptSignal.removeEventListener("abort", onAbort)
        promptClosed = true
      })
    },
  })
  let runSettled = false
  run.then(() => { runSettled = true }, () => { runSettled = true })
  const failedWithoutVerdict = assert.rejects(run, /no acceptance verdict/)
  await ownerActionStarted

  const boundedTimerWasScheduled = typeof timerCallback === "function"
  monotonicNow = actionBoundMs
  const stillWaitingAtActionBound = !runSettled
  if (boundedTimerWasScheduled) timerCallback()
  else controller.abort()
  await failedWithoutVerdict

  assert.equal(stillWaitingAtActionBound, true)
  assert.equal(promptClosed, true)
  assert.equal(clientClosed, true)
  signal.removeEventListener("abort", recordRunSignalAbort)
  assert.equal(runSignalAbortEvents, 0)
  assert.equal(getEventListeners(signal, "abort").length, 0)
  assert.equal(boundedTimerWasScheduled, true)
  assert.equal(controller.signal.aborted, false)
  assert.equal(promptSignal.aborted, true)
  assert.equal(getEventListeners(promptSignal, "abort").length, 0)
  assert.equal(timerDelay, actionBoundMs)
  assert.equal(timerCleared, true)
  const lifecycle = product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
  assert.equal(lifecycle.length, 1)
  assert.deepEqual(lifecycle.map(([, request]) => [request.environmentId, request.action]),
    [["environment-fixture-1", "delete"]])
  assert.equal(product.calls.filter(([name]) => name === "CreateManagedEnvironment").length, 1)
  const incomplete = JSON.parse(await readFile(output, "utf8"))
  assert.equal("verdict" in incomplete, false)
  assert.equal("passed" in incomplete, false)
  assert.deepEqual(incomplete.requiredUserActions.map(({ action }) => action), ["start_agent_via_normal_path"])
  assert.equal(incomplete.operations.find((item) => item.kind === "delete")?.status, "succeeded")
  assert.equal(incomplete.operations.some((item) => item.kind === "stop"), false)
})

for (const [scenario, expectedActions] of [
  ["shutdown_disabled", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path"]],
  ["shutdown_keep_running", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "keep_running_via_cloud_ui"]],
]) {
  test(`MP-09 ${scenario} observes no STOP through its required window`, async (t) => {
    const root = await scratch(t)
    const output = join(root, `${scenario}.json`)
    const policy = SHUTDOWN_SCENARIOS[scenario].policy
    const product = fakeProductPath({ policy })
    const pauses = []
    let monotonicNow = 0
    const requestedActions = []
    const result = await runManagedShutdownTrigger(parseArguments(argumentsFor(output, scenario)), {
      client: { send: product.send, close: async () => {} },
      requests: product.requests,
      send: product.send,
      id: () => `${scenario}-fixture`,
      now: () => new Date(Date.parse(baseTime) + monotonicNow),
      monotonic: () => monotonicNow,
      pause: async (milliseconds) => {
        pauses.push(milliseconds)
        monotonicNow += milliseconds
      },
      ask: async (message) => {
        if (message.includes("start exactly one agent")) {
          requestedActions.push("start_agent_via_normal_path")
          product.updateCurrent({
            runningAgentCount: 1,
            lastActivityReportedAt: atSecond(1),
            lastActivityChangedAt: atSecond(1),
            updatedAt: atSecond(1),
          })
        } else if (message.includes("Finish that agent")) {
          requestedActions.push("finish_agent_via_normal_provider_path")
          const idleAt = atSecond(2)
          const deadlineTime = policy.idleDelaySeconds === null
            ? null
            : Math.max(
              Date.parse(baseTime) + policy.minimumRuntimeSeconds * 1_000,
              Date.parse(idleAt) + policy.idleDelaySeconds * 1_000,
              Date.parse(idleAt) + MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000,
            )
          const deadline = deadlineTime === null ? null : new Date(deadlineTime).toISOString()
          product.updateCurrent({
            runningAgentCount: 0,
            lastActivityReportedAt: idleAt,
            lastActivityChangedAt: idleAt,
            autoStopDeadlineAt: deadline,
            autoStopWarningAt: deadline === null
              ? null
              : new Date(Date.parse(deadline) - MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000).toISOString(),
            updatedAt: idleAt,
          })
        } else if (message.includes("Keep running")) {
          requestedActions.push("keep_running_via_cloud_ui")
          product.updateCurrent({ autoStopDeadlineAt: null, autoStopWarningAt: null, updatedAt: atSecond(3) })
        } else {
          assert.fail(`unexpected owner action prompt: ${message}`)
        }
      },
    })

    assert.deepEqual(requestedActions, expectedActions)
    const expectedUntil = scenario === "shutdown_keep_running" ? 1_022_000 : 120_000
    assert.equal(monotonicNow, expectedUntil)
    assert.equal(pauses.reduce((sum, value) => sum + value, 0), expectedUntil)
    const finalReady = result.observations.findLast(({ environment: observed }) => observed.observedState === "ready")
    assert.ok(finalReady)
    assert.equal(finalReady.capturedAt, atSecond(expectedUntil / 1_000))
    assert.ok(result.snapshotCoverage.workflow.samples >= pauses.length)
    assert.equal(result.snapshotCoverage.workflow.lastCompletedAt, finalReady.capturedAt)
    assert.ok(result.requiredUserActions.every(action => typeof action.acknowledgedAt === "string"))
    assert.equal(result.operations.some(({ kind }) => kind === "stop"), false)
    assert.deepEqual(product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
      .map(([, request]) => request.action), ["delete"])
    assert.equal(JSON.parse(await readFile(output, "utf8")).schema, result.schema)
    assert.equal("verdict" in result, false)
  })
}

for (const [scenario, expectedActions] of [
  ["shutdown_agents_done", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path"]],
  ["shutdown_restart_reconciliation", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "restart_cloud_auto_stop_reconciliation"]],
  ["shutdown_all_clients_disconnected", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "disconnect_all_clients_from_managed_environment"]],
]) {
  test(`${scenario} observes the exact eventual automatic STOP receipt`, async (t) => {
    const root = await scratch(t)
    const output = join(root, `${scenario}.json`)
    const policy = SHUTDOWN_SCENARIOS[scenario].policy
    const product = fakeProductPath({ policy })
    let monotonicNow = 0
    const requestedActions = []
    const result = await runManagedShutdownTrigger(parseArguments(argumentsFor(output, scenario)), {
      client: { send: product.send, close: async () => {} },
      requests: product.requests,
      send: product.send,
      id: () => `${scenario}-fixture`,
      now: () => new Date(Date.parse(baseTime) + monotonicNow),
      monotonic: () => monotonicNow,
      pause: async (milliseconds) => {
        monotonicNow += milliseconds
        product.advanceAutomaticStop(new Date(Date.parse(baseTime) + monotonicNow).toISOString())
      },
      ask: async (message) => {
        if (message.includes("start exactly one agent")) {
          requestedActions.push("start_agent_via_normal_path")
          product.updateCurrent({
            runningAgentCount: 1,
            lastActivityReportedAt: atSecond(1),
            lastActivityChangedAt: atSecond(1),
            updatedAt: atSecond(1),
          })
        } else if (message.includes("Finish that agent")) {
          requestedActions.push("finish_agent_via_normal_provider_path")
          const idleAt = atSecond(2)
          const deadline = new Date(Math.max(
            Date.parse(baseTime) + policy.minimumRuntimeSeconds * 1_000,
            Date.parse(idleAt) + policy.idleDelaySeconds * 1_000,
            Date.parse(idleAt) + MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000,
          )).toISOString()
          product.updateCurrent({
            runningAgentCount: 0,
            lastActivityReportedAt: idleAt,
            lastActivityChangedAt: idleAt,
            autoStopDeadlineAt: deadline,
            autoStopWarningAt: new Date(Date.parse(deadline)
              - MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000).toISOString(),
            updatedAt: idleAt,
          })
        } else if (message.includes("restart auto-stop reconciliation")) {
          requestedActions.push("restart_cloud_auto_stop_reconciliation")
        } else if (message.includes("Disconnect every interactive Chariox client")) {
          requestedActions.push("disconnect_all_clients_from_managed_environment")
        } else {
          assert.fail(`unexpected owner action prompt: ${message}`)
        }
      },
    })

    assert.deepEqual(requestedActions, expectedActions)
    const automaticStop = result.operations.find(({ kind }) => kind === "stop")
    assert.ok(automaticStop)
    assert.equal(automaticStop.status, "succeeded")
    assert.equal(automaticStop.createdAt, atSecond(902))
    assert.equal(automaticStop.desiredRevision, 2)
    assert.equal(monotonicNow >= 902_000, true)
    assert.equal(result.observations.some(({ environment: observed }) => observed.observedState === "stopped"), true)
    assert.equal(result.requiredUserActions.some(({ action }) => action === expectedActions.at(-1)), true)
    assert.equal("verdict" in result, false)
  })
}

for (const [scenario, expectedActions] of [
  ["shutdown_agents_done", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path"]],
  ["shutdown_restart_reconciliation", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "restart_cloud_auto_stop_reconciliation"]],
  ["shutdown_all_clients_disconnected", ["start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "disconnect_all_clients_from_managed_environment"]],
]) {
  test(`${scenario} with no automatic STOP expires within its bound and still deletes`, async (t) => {
    const root = await scratch(t)
    const output = join(root, `${scenario}-never-stops.json`)
    const options = parseArguments(argumentsFor(output, scenario))
    const actionBoundMs = options.maxBillableSeconds * 1_000 - SHUTDOWN_TRIGGER_LIMITS.cleanupReserveMs
    const product = fakeProductPath({ policy: options.descriptor.policy })
    let monotonicNow = 0
    const requestedActions = []
    await assert.rejects(runManagedShutdownTrigger(options, {
      client: { send: product.send, close: async () => {} },
      requests: product.requests,
      send: product.send,
      id: () => `${scenario}-never-stops-fixture`,
      now: () => new Date(Date.parse(baseTime) + monotonicNow),
      monotonic: () => monotonicNow,
      pause: async (milliseconds) => { monotonicNow += milliseconds },
      ask: async (message) => {
        if (message.includes("start exactly one agent")) {
          requestedActions.push("start_agent_via_normal_path")
          product.updateCurrent({
            runningAgentCount: 1,
            lastActivityReportedAt: atSecond(1),
            lastActivityChangedAt: atSecond(1),
            updatedAt: atSecond(1),
          })
        } else if (message.includes("Finish that agent")) {
          requestedActions.push("finish_agent_via_normal_provider_path")
          const idleAt = atSecond(2)
          const deadline = new Date(Math.max(
            Date.parse(baseTime) + options.descriptor.policy.minimumRuntimeSeconds * 1_000,
            Date.parse(idleAt) + options.descriptor.policy.idleDelaySeconds * 1_000,
            Date.parse(idleAt) + MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000,
          )).toISOString()
          product.updateCurrent({
            runningAgentCount: 0,
            lastActivityReportedAt: idleAt,
            lastActivityChangedAt: idleAt,
            autoStopDeadlineAt: deadline,
            autoStopWarningAt: new Date(Date.parse(deadline)
              - MANAGED_SHUTDOWN_WARNING_SECONDS * 1_000).toISOString(),
            updatedAt: idleAt,
          })
        } else if (message.includes("restart auto-stop reconciliation")) {
          requestedActions.push("restart_cloud_auto_stop_reconciliation")
        } else if (message.includes("Disconnect every interactive Chariox client")) {
          requestedActions.push("disconnect_all_clients_from_managed_environment")
        } else {
          assert.fail(`unexpected owner action prompt: ${message}`)
        }
      },
    }), /no acceptance verdict/)

    assert.equal(monotonicNow, actionBoundMs)
    assert.deepEqual(requestedActions, expectedActions)
    assert.deepEqual(product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
      .map(([, request]) => request.action), ["delete"])
    const incomplete = JSON.parse(await readFile(output, "utf8"))
    assert.equal("verdict" in incomplete, false)
    assert.equal("passed" in incomplete, false)
    assert.equal(incomplete.operations.some(({ kind }) => kind === "stop"), false)
    assert.equal(incomplete.operations.find(({ kind }) => kind === "delete")?.status, "succeeded")
    assert.deepEqual(incomplete.requiredUserActions.map(({ action }) => action), expectedActions)
  })
}

test("three-hour minimum scenario observes through runtime start plus three hours and then the automatic stop", async (t) => {
  const root = await scratch(t)
  const output = join(root, "minimum-runtime.json")
  const scenario = "shutdown_minimum_3h"
  const policy = SHUTDOWN_SCENARIOS[scenario].policy
  const product = fakeProductPath({ policy })
  let monotonicNow = 0
  const result = await runManagedShutdownTrigger(parseArguments(argumentsFor(output, scenario)), {
    client: { send: product.send, close: async () => {} },
    requests: product.requests,
    send: product.send,
    id: () => "minimum-runtime-fixture",
    now: () => new Date(Date.parse(baseTime) + monotonicNow),
    monotonic: () => monotonicNow,
    pause: async (milliseconds) => {
      monotonicNow += milliseconds
      product.advanceAutomaticStop(new Date(Date.parse(baseTime) + monotonicNow).toISOString())
    },
    ask: async (message) => {
      if (message.includes("start exactly one agent")) {
        product.updateCurrent({
          runningAgentCount: 1,
          lastActivityReportedAt: atSecond(1),
          lastActivityChangedAt: atSecond(1),
          updatedAt: atSecond(1),
        })
      } else if (message.includes("Finish that agent")) {
        const idleAt = atSecond(2)
        const deadline = atSecond(10_800)
        product.updateCurrent({
          runningAgentCount: 0,
          lastActivityReportedAt: idleAt,
          lastActivityChangedAt: idleAt,
          autoStopDeadlineAt: deadline,
          autoStopWarningAt: atSecond(10_800 - MANAGED_SHUTDOWN_WARNING_SECONDS),
          updatedAt: idleAt,
        })
      } else {
        assert.fail(`unexpected owner action prompt: ${message}`)
      }
    },
  })

  const lastReady = result.observations.filter(({ environment: observed }) => observed.observedState === "ready")
    .findLast(({ capturedAt }) => Date.parse(capturedAt) < Date.parse(atSecond(10_800)))
  const automaticStop = result.operations.find(({ kind }) => kind === "stop")
  assert.ok(lastReady)
  assert.ok(Date.parse(lastReady.capturedAt) >= Date.parse(atSecond(10_800 - 60)))
  assert.equal(automaticStop?.createdAt, atSecond(10_800))
  assert.equal(monotonicNow, 10_800_000)
  assert.deepEqual(result.requestedAutoStopPolicy, { minimumRuntimeSeconds: 10_800, idleDelaySeconds: 900 })
  assert.equal("verdict" in result, false)
})

test("interrupting a required user action still deletes the one created target", async (t) => {
  const root = await scratch(t)
  const output = join(root, "interrupted.json")
  const policy = { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }
  const product = fakeProductPath({ policy })
  const controller = new AbortController()
  let closed = false
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(output, "shutdown_agents_done")), {
    client: { send: product.send, close: async () => { closed = true } },
    requests: product.requests,
    send: product.send,
    pause: async () => {},
    id: () => "run-interrupted",
    signal: controller.signal,
    ask: () => {
      controller.abort()
      return new Promise(() => {})
    },
  }), /no acceptance verdict/)
  assert.equal(closed, true)
  const actions = product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
    .map(([, body]) => body.action)
  assert.deepEqual(actions, ["delete"])
  const incomplete = JSON.parse(await readFile(output, "utf8"))
  assert.equal(incomplete.target.environmentId, "environment-fixture-1")
  assert.equal("verdict" in incomplete, false)
  assert.equal(incomplete.operations.some((item) => item.kind === "delete" && item.status === "succeeded"), true)
  assert.deepEqual(incomplete.requiredUserActions.map(({ action }) => action), ["start_agent_via_normal_path"])
  assert.equal(incomplete.requiredUserActions[0].completedAt, undefined)
})

test('MP-09 failed capture retains safe stage diagnostics without external error contents', async t => {
  const root = await scratch(t), output = join(root, 'diagnostic.json'), product = fakeProductPath({ includeHistory: false })
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(output)), {
    client: { send: product.send, close: async () => {} }, requests: product.requests, send: product.send,
    pause: async () => {}, id: () => 'run-diagnostic',
  }), /no acceptance verdict/)
  const capture = JSON.parse(await readFile(output, 'utf8'))
  assert.equal(capture.failures[0].stage, 'workflow')
  assert.equal(capture.failures[0].invariant, 'managed operation history is unavailable')
  assert.equal(capture.failures[1].stage, 'cleanup')
  assert.equal('passed' in capture, false)
})

test('MP-09 failure diagnostics discard external messages, stacks and token-like codes', async t => {
  const { recordShutdownFailure } = await import('./lib/managed-shutdown-trigger-failure.mjs')
  const capture = {}, secret = 'fixture-sensitive-token-must-never-enter-evidence'
  recordShutdownFailure(capture, 'workflow', Object.assign(new Error(secret), { code: secret, name: secret }), baseTime)
  assert.equal(JSON.stringify(capture).includes(secret), false)
  assert.equal(capture.failures[0].invariant, 'unclassified_failure')
  assert.equal(capture.failures[0].code, null)
})


test("MP-09 exact delete waits through Cloud failed attempt until same operation succeeds", async (t) => {
  const root = await scratch(t)
  const product = fakeProductPath()
  let deleteStarted = false, observedFailed = false
  const send = async request => {
    const response = await product.send(request)
    if (request.RequestManagedEnvironmentLifecycle?.action === "delete") deleteStarted = true
    if (request.GetManagedEnvironment && deleteStarted) {
      const details = structuredClone(response.ManagedEnvironment)
      const deletion = details.operations.find(operation => operation.kind === "delete")
      if (!observedFailed) {
        observedFailed = true
        deletion.status = "failed"
        details.environment.observedState = "failed"
        details.environment.observedRevision -= 1
      } else deletion.attempt = 2
      return { ManagedEnvironment: details }
    }
    return response
  }
  const result = await runManagedShutdownTrigger(parseArguments(argumentsFor(join(root, "retry.json"))), {
    client: { send, close: async () => {} }, requests: product.requests, send,
    pause: async () => {}, id: () => "run-retry-fixture",
  })
  assert.equal(observedFailed, true)
  assert.equal(result.operations.find(operation => operation.kind === "delete").status, "succeeded")
  assert.equal(result.operations.find(operation => operation.kind === "delete").attempt, 2)
  assert.equal(result.observations.at(-1).environment.observedState, "deleted")
  assert.equal(result.observations.some(row => row.environment.observedState === "failed"), true)
})


test("MP-09 permanently failed exact delete still expires without an acceptance verdict", async (t) => {
  const root = await scratch(t)
  const output = join(root, "permanent-delete-failure.json")
  const product = fakeProductPath()
  let elapsed = 0, deleteStarted = false, failedReads = 0
  const send = async request => {
    const response = await product.send(request)
    if (request.RequestManagedEnvironmentLifecycle?.action === "delete") deleteStarted = true
    if (request.GetManagedEnvironment && deleteStarted) {
      failedReads += 1
      const details = structuredClone(response.ManagedEnvironment)
      details.operations.find(operation => operation.kind === "delete").status = "failed"
      details.environment.observedState = "failed"
      details.environment.observedRevision -= 1
      return { ManagedEnvironment: details }
    }
    return response
  }
  await assert.rejects(runManagedShutdownTrigger(parseArguments(argumentsFor(output)), {
    client: { send, close: async () => {} }, requests: product.requests, send,
    pause: async milliseconds => { elapsed += milliseconds }, monotonic: () => elapsed,
    now: () => new Date(Date.parse(baseTime) + elapsed), id: () => "permanent-retry-fixture",
  }), /no acceptance verdict/)
  const capture = JSON.parse(await readFile(output, "utf8"))
  assert.ok(failedReads > 1 && failedReads <= 100)
  assert.equal(elapsed, 600_000)
  assert.equal(capture.operations.find(operation => operation.kind === "delete").status, "failed")
  assert.equal(capture.failures.at(-1).stage, "cleanup")
  assert.equal(capture.observations.some(row => row.environment.observedState === "deleted"), false)
})

// MP-09/MP-10: this clock and owner-IPC fixture never establishes live VM timing.
async function shutdownFixture(t, scenario, hooks = {}) {
  const root = await scratch(t)
  const output = join(root, "observation.json")
  const options = parseArguments(argumentsFor(output, scenario))
  const product = fakeProductPath({ policy: options.descriptor.policy })
  let elapsed = 0
  const deps = {
    client: { send: request => deps.send(request), close: async () => {} },
    requests: product.requests,
    send: async request => {
      const response = await product.send(request)
      return hooks.read ? hooks.read(request, response, elapsed, product) : response
    },
    id: () => `${scenario}-fixture`,
    monotonic: () => elapsed,
    now: () => new Date(Date.parse(baseTime) + elapsed),
    pause: async milliseconds => {
      elapsed += milliseconds
      product.advanceAutomaticStop(atSecond(elapsed / 1_000))
    },
    ask: async message => {
      if (message.includes("start exactly one agent")) {
        product.updateCurrent({ runningAgentCount: 1, lastActivityReportedAt: atSecond(1),
          lastActivityChangedAt: atSecond(1), updatedAt: atSecond(1) })
      } else if (message.includes("Finish that agent")) {
        const delay = options.descriptor.policy.idleDelaySeconds
        const deadline = delay === null ? null : atSecond(Math.max(
          options.descriptor.policy.minimumRuntimeSeconds, 2 + delay, 2 + MANAGED_SHUTDOWN_WARNING_SECONDS))
        product.updateCurrent({ runningAgentCount: 0, lastActivityReportedAt: atSecond(2),
          lastActivityChangedAt: atSecond(2), updatedAt: atSecond(2), autoStopDeadlineAt: deadline,
          autoStopWarningAt: deadline === null ? null : atSecond((Date.parse(deadline) - Date.parse(baseTime)) / 1_000 - MANAGED_SHUTDOWN_WARNING_SECONDS) })
      } else if (message.includes("Keep running")) {
        product.updateCurrent({ autoStopDeadlineAt: null, autoStopWarningAt: null })
      } else if (message.includes("Stop this disposable environment")) {
        await product.send(product.requests.requestManagedEnvironmentLifecycleRequest({
          environmentId: "environment-fixture-1", action: "stop", idempotencyKey: "fixture-ui-stop" }))
      } else if (!message.includes("signed managed-kernel deployment")) {
        assert.fail(`unexpected fixture action: ${message}`)
      }
    },
    ...hooks.deps,
  }
  return { product, output, options, deps, elapsed: () => elapsed,
    run: () => runManagedShutdownTrigger(options, deps) }
}

for (const scenario of ["shutdown_idle_15m", "shutdown_idle_30m", "shutdown_custom", "shutdown_deployment_reconciliation"]) {
  test(`MP-09/MP-10 ${scenario} retains exact deadline, binding and STOP-before-DELETE observations`, async t => {
    const f = await shutdownFixture(t, scenario)
    const capture = await f.run()
    const stop = capture.operations.find(operation => operation.kind === "stop")
    assert.equal(stop.createdAt, atSecond(2 + f.options.descriptor.policy.idleDelaySeconds))
    assert.equal(stop.status, "succeeded")
    assert.equal(stop.environmentId, capture.target.environmentId)
    assert.equal(capture.operations.at(-1).kind, "delete")
    assert.equal(capture.snapshotCoverage.workflow.maximumGapMs, 10_000)
    assert.ok(capture.snapshotCoverage.workflow.samples > 50)
    assert.equal(capture.requiredUserActions.every(action => action.acknowledgedAt), true)
    assert.equal("passed" in capture, false)
  })
}

test("MP-09 manual UI fixture retains disabled default policy and acknowledgement separately from STOP receipt", async t => {
  const f = await shutdownFixture(t, "shutdown_manual")
  const capture = await f.run()
  assert.deepEqual(capture.requestedAutoStopPolicy, { minimumRuntimeSeconds: 0, idleDelaySeconds: null })
  assert.deepEqual(capture.requiredUserActions.map(action => action.action), ["manual_stop_via_cloud_ui"])
  assert.equal(capture.requiredUserActions[0].acknowledgedAt, baseTime)
  assert.deepEqual(capture.operations.map(operation => operation.kind), ["create", "stop", "delete"])
  assert.equal("passed" in capture, false)
})

for (const scenario of ["shutdown_agents_done", "shutdown_manual", "shutdown_explicit_lifecycle_reconciliation"]) {
  test(`MP-09 ${scenario} STOP waits through exactly one failed attempt and retains both receipts`, async t => {
    let failed = false
    const f = await shutdownFixture(t, scenario, {
      read(request, response) {
        if (!request.GetManagedEnvironment) return response
        const details = structuredClone(response.ManagedEnvironment)
        const stop = details.operations.find(operation => operation.kind === "stop")
        if (!stop) return response
        if (!failed) {
          failed = true
          stop.status = "failed"
          stop.completedAt = null
          details.environment.observedState = "failed"
          details.environment.observedRevision -= 1
        } else stop.attempt = 2
        return { ManagedEnvironment: details }
      },
    })
    const capture = await f.run()
    const transitions = capture.operationObservations.filter(row => row.operation.kind === "stop")
    assert.deepEqual(transitions.map(row => [row.operation.operationId, row.operation.status, row.operation.attempt]),
      [[scenario === "shutdown_agents_done" ? "auto-stop-op-2" : "stop-op-2", "failed", 1],
        [scenario === "shutdown_agents_done" ? "auto-stop-op-2" : "stop-op-2", "succeeded", 2]])
    assert.equal(capture.operations.filter(operation => operation.kind === "stop").length, 1)
    assert.deepEqual(f.product.calls.filter(([name]) => name === "RequestManagedEnvironmentLifecycle")
      .map(([, request]) => request.action), scenario === "shutdown_agents_done" ? ["delete"] : ["stop", "delete"])
  })
}

test("MP-09 retry cannot replace the initially observed automatic STOP operation", async t => {
  let observed = false
  const f = await shutdownFixture(t, "shutdown_agents_done", {
    read(request, response) {
      if (!request.GetManagedEnvironment) return response
      const details = structuredClone(response.ManagedEnvironment)
      const stop = details.operations.find(operation => operation.kind === "stop")
      if (!stop) return response
      if (!observed) {
        observed = true
        stop.status = "failed"
        stop.completedAt = null
      } else stop.operationId = "replacement-stop"
      return { ManagedEnvironment: details }
    },
  })
  await assert.rejects(f.run(), /no acceptance verdict/)
  const capture = JSON.parse(await readFile(f.output, "utf8"))
  assert.equal(capture.failures[0].invariant, "exact managed operation is unavailable or ambiguous")
  assert.equal(capture.operations.at(-1).kind, "delete")
})

test("MP-09 even a pending automatic STOP created before the deadline fails immediately", async t => {
  const f = await shutdownFixture(t, "shutdown_agents_done", {
    read(request, response, elapsed) {
      if (!request.GetManagedEnvironment || elapsed < 10_000 || elapsed > 20_000) return response
      const details = structuredClone(response.ManagedEnvironment)
      if (details.environment.observedState !== "ready") return response
      details.operations.push(operation("early-pending-stop", "stop", 2, "pending", atSecond(2)))
      return { ManagedEnvironment: details }
    },
  })
  await assert.rejects(f.run(), /no acceptance verdict/)
  assert.equal(f.elapsed(), 10_000)
  const capture = JSON.parse(await readFile(f.output, "utf8"))
  assert.equal(capture.failures[0].invariant, "automatic stop operation is incomplete or out of order")
  assert.equal(capture.operations.at(-1).kind, "delete")
})

test("MP-09 Keep running cannot hide an obsolete STOP after its former deadline", async t => {
  const f = await shutdownFixture(t, "shutdown_keep_running", {
    read(request, response, elapsed) {
      if (!request.GetManagedEnvironment || elapsed < 902_000) return response
      const details = structuredClone(response.ManagedEnvironment)
      if (details.environment.observedState === "ready") {
        details.operations.push(operation("obsolete-stop", "stop", 2, "pending", atSecond(902)))
      }
      return { ManagedEnvironment: details }
    },
  })
  await assert.rejects(f.run(), /no acceptance verdict/)
  const capture = JSON.parse(await readFile(f.output, "utf8"))
  assert.equal(capture.failures[0].invariant, "a stop operation appeared after Keep running")
})

test("MP-09/MP-10 reviewed observation barrier runs after STOP and before normal DELETE", async t => {
  const f = await shutdownFixture(t, "shutdown_explicit_lifecycle_reconciliation")
  let inspected = false
  f.deps.observeBeforeCleanup = async ({ target, scenario, signal, remainingMs }) => {
    assert.equal(scenario, f.options.scenario)
    assert.equal(signal.aborted, false)
    assert.ok(remainingMs() > 0)
    const result = await f.product.send(f.product.requests.getManagedEnvironmentRequest(target.environmentId))
    assert.equal(result.ManagedEnvironment.environment.observedState, "stopped")
    assert.equal(f.product.operations.some(operation => operation.kind === "delete"), false)
    inspected = true
    // Independent receipts are retained externally by the bridge, not trusted here.
    return { passed: true, sensitive: "fixture-marker-not-retained" }
  }
  const capture = await f.run()
  assert.equal(inspected, true)
  assert.equal(capture.beforeCleanupObservation.completedAt, baseTime)
  assert.equal(JSON.stringify(capture).includes("fixture-marker-not-retained"), false)
  assert.equal(capture.operations.at(-1).kind, "delete")
})

test("MP-09/MP-10 expired observation barrier still reserves normal DELETE cleanup", async t => {
  const f = await shutdownFixture(t, "shutdown_explicit_lifecycle_reconciliation")
  let elapsed = 0, aborted = false
  f.deps.monotonic = () => elapsed
  f.deps.now = () => new Date(Date.parse(baseTime) + elapsed)
  f.deps.setTimeout = (callback, milliseconds) => {
    queueMicrotask(() => { elapsed += milliseconds; callback() })
    return 1
  }
  f.deps.clearTimeout = () => {}
  f.deps.observeBeforeCleanup = ({ signal }) => new Promise((_, reject) => {
    signal.addEventListener("abort", () => { aborted = true; reject(new Error("fixture observation aborted")) }, { once: true })
  })
  await assert.rejects(f.run(), /no acceptance verdict/)
  assert.equal(aborted, true)
  assert.equal(elapsed, 300_000)
  const capture = JSON.parse(await readFile(f.output, "utf8"))
  assert.equal(capture.beforeCleanupObservation.completedAt, undefined)
  assert.equal(capture.operations.at(-1).kind, "delete")
  assert.equal(capture.operations.at(-1).status, "succeeded")
})

test("MP-09/MP-10 deduplicated reads retain sampling gaps and reject a regressing observer clock", async () => {
  const { recordSnapshotCoverage } = await import("./lib/managed-shutdown-trigger-observation.mjs")
  const capture = {}
  recordSnapshotCoverage(capture, "workflow", baseTime, 0, 2)
  recordSnapshotCoverage(capture, "workflow", atSecond(100), 99_990, 100_000)
  assert.equal(capture.snapshotCoverage.workflow.maximumGapMs, 99_998)
  assert.equal(capture.snapshotCoverage.workflow.maximumRequestMs, 10)
  assert.throws(() => recordSnapshotCoverage(capture, "workflow", atSecond(90), 89_990, 90_000), /clock regressed/)
  assert.equal(capture.snapshotCoverage.workflow.samples, 2)
})

for (const [minimum, delay, changed, reported, deadline] of [
  [0, 0, 2, 2, 32],
  [0, 900, 2, 2, 902],
  [0, 1800, 2, 2, 1802],
  [10800, 900, 2, 2, 10800],
  [3600, 600, 4000, 4001, 4600],
  [0, 0, 2, 50, 80],
]) {
  test(`MP-09 exact policy ${minimum}/${delay} preserves final finish and latest-report warning floor`, () => {
    const summary = projectSummary({
      ...environment({ policy: { minimumRuntimeSeconds: minimum, idleDelaySeconds: delay } }),
      lastActivityChangedAt: atSecond(changed), lastActivityReportedAt: atSecond(reported),
      autoStopDeadlineAt: atSecond(deadline),
      autoStopWarningAt: atSecond(deadline - MANAGED_SHUTDOWN_WARNING_SECONDS),
    })
    assert.equal(verifyIdleDeadline(summary), atSecond(deadline))
    assert.throws(() => verifyIdleDeadline({ ...summary, autoStopDeadlineAt: atSecond(deadline - 1) }), /Cloud deadline/)
  })
}

test("MP-09 renewed activity cannot be credited as the original final-agent STOP", async t => {
  const f = await shutdownFixture(t, "shutdown_agents_done", {
    read(request, response, elapsed) {
      if (!request.GetManagedEnvironment || elapsed < 10_000) return response
      const details = structuredClone(response.ManagedEnvironment)
      if (details.environment.observedState === "ready") {
        Object.assign(details.environment, { runningAgentCount: 1, lastActivityChangedAt: atSecond(10),
          lastActivityReportedAt: atSecond(10), autoStopDeadlineAt: null, autoStopWarningAt: null })
      }
      return { ManagedEnvironment: details }
    },
  })
  await assert.rejects(f.run(), /no acceptance verdict/)
  const capture = JSON.parse(await readFile(f.output, "utf8"))
  assert.equal(capture.failures[0].invariant, "automatic stop no longer matches the idle activity transition")
  assert.equal(capture.operations.at(-1).kind, "delete")
})

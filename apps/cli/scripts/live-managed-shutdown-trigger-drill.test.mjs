import assert from "node:assert/strict"
import { getEventListeners } from "node:events"
import { mkdtemp, readFile, realpath, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { parseArguments, runManagedShutdownTrigger, SHUTDOWN_SCENARIOS } from "./live-managed-shutdown-trigger-drill.mjs"
import { MANAGED_SHUTDOWN_WARNING_SECONDS } from "./lib/managed-shutdown-trigger-config.mjs"
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
  const product = fakeProductPath({ policy: SHUTDOWN_SCENARIOS.shutdown_agents_done.policy })
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
  const run = runManagedShutdownTrigger(parseArguments(argumentsFor(output, "shutdown_agents_done")), {
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
  monotonicNow = 300_000
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
  assert.equal(timerDelay, 300_000)
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
  test(`${scenario} retains its no-stop negative at the existing observation deadline`, async (t) => {
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
    assert.equal(monotonicNow, 120_000)
    assert.deepEqual(pauses, Array(12).fill(10_000))
    const finalReady = result.observations.findLast(({ environment: observed }) => observed.observedState === "ready")
    assert.ok(finalReady)
    assert.equal(finalReady.capturedAt, atSecond(120))
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

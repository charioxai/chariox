#!/usr/bin/env node

import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const pageSize = 100
const maxHistoryActions = 4096
const defaultObserveMs = 60_000
const maxObserveMs = 300_000
const minPollMs = 250
const maxPollMs = 10_000

function parseArgs(argv) {
  const options = {
    kernelUrl: null,
    sessionId: null,
    output: null,
    observeMs: defaultObserveMs,
    pollMs: 1_000,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const value = () => {
      const next = argv[index + 1]
      if (!next || next.startsWith("--")) throw new Error(`missing value for ${arg}`)
      index += 1
      return next
    }
    if (arg === "--kernel-url") options.kernelUrl = value()
    else if (arg === "--session") options.sessionId = value()
    else if (arg === "--output") options.output = path.resolve(value())
    else if (arg === "--observe-ms") options.observeMs = Number(value())
    else if (arg === "--poll-ms") options.pollMs = Number(value())
    else if (arg === "--help" || arg === "-h") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function validateOptions(options) {
  assert.ok(options.kernelUrl, "--kernel-url is required")
  assert.ok(options.sessionId, "--session is required")
  const endpoint = new URL(options.kernelUrl)
  assert.ok(["ws:", "wss:"].includes(endpoint.protocol), "--kernel-url must be a WebSocket URL")
  assert.ok(Number.isSafeInteger(options.observeMs) && options.observeMs >= 0 && options.observeMs <= maxObserveMs,
    `--observe-ms must be between 0 and ${maxObserveMs}`)
  assert.ok(Number.isSafeInteger(options.pollMs) && options.pollMs >= minPollMs && options.pollMs <= maxPollMs,
    `--poll-ms must be between ${minPollMs} and ${maxPollMs}`)
}

function helpText() {
  return [
    "Usage: node apps/cli/scripts/live-browser-computer-drill-e.mjs --kernel-url ws://HOST:PORT --session SESSION_ID [options]",
    "",
    "Attach read-only to an existing Room before the Drill E agents begin.",
    "This checker does not create a Room, submit Actions, or request takeover.",
    "",
    "Options:",
    `  --observe-ms N  Observe snapshots for 0..${maxObserveMs} ms (default ${defaultObserveMs})`,
    `  --poll-ms N     Snapshot interval ${minPollMs}..${maxPollMs} ms (default 1000)`,
    "  --output PATH   Write the redacted report outside the repository",
  ].join("\n")
}

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && Object.hasOwn(response, variant),
    `kernel response omitted ${variant}`)
  return response[variant]
}

function safeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0
}

function assertEnvironment(environment, sessionId, expectedIdentity = null) {
  assert.ok(environment && typeof environment === "object", "kernel Environment snapshot is missing")
  assert.equal(environment.session_id, sessionId, "kernel returned a different Room")
  assert.ok(typeof environment.environment_id === "string" && environment.environment_id,
    "kernel Environment snapshot omitted its identity")
  assert.ok(safeInteger(environment.runtime_generation), "kernel Environment runtime generation is invalid")
  assert.ok(safeInteger(environment.event_cursor), "kernel Environment event cursor is invalid")
  assert.equal(environment.lifecycle, "ready", "Room Environment is not ready")
  assert.ok(environment.viewport && safeInteger(environment.viewport.revision),
    "kernel Environment snapshot omitted its canonical viewport")
  assert.ok(Array.isArray(environment.health) && Array.isArray(environment.actors)
    && Array.isArray(environment.pointers) && Array.isArray(environment.tabs)
    && Array.isArray(environment.actions) && Array.isArray(environment.pending_input_takeovers)
    && Array.isArray(environment.input_ownership), "kernel Environment snapshot is malformed")
  assert.ok(environment.focused_tab_id === null || typeof environment.focused_tab_id === "string",
    "kernel Environment focused tab identity is malformed")
  const actorIds = new Set()
  for (const actor of environment.actors) {
    assert.ok(actor && typeof actor.actor_id === "string" && actor.actor_id
      && ["human", "agent"].includes(actor.kind)
      && typeof actor.display_label === "string"
      && ["blue", "cyan", "green", "amber", "orange", "rose", "violet", "slate"].includes(actor.presentation_color)
      && ["present", "away", "disconnected"].includes(actor.presence),
    "kernel Environment contains a malformed Actor projection")
    assert.ok(!actorIds.has(actor.actor_id), "kernel Environment contains a duplicate Actor identity")
    actorIds.add(actor.actor_id)
  }
  const tabIds = new Set()
  for (const tab of environment.tabs) {
    assert.ok(tab && typeof tab.tab_id === "string" && tab.tab_id
      && typeof tab.url === "string" && typeof tab.title === "string"
      && safeInteger(tab.document_revision) && typeof tab.focused === "boolean",
    "kernel Environment contains a malformed Tab projection")
    assert.ok(!tabIds.has(tab.tab_id), "kernel Environment contains a duplicate Tab identity")
    tabIds.add(tab.tab_id)
  }
  assert.ok(environment.focused_tab_id === null || tabIds.has(environment.focused_tab_id),
    "kernel Environment focused Tab is absent from the Tab projection")
  if (expectedIdentity) {
    assert.equal(environment.environment_id, expectedIdentity.environmentId,
      "Room Environment identity changed during Drill E")
    assert.equal(environment.runtime_generation, expectedIdentity.runtimeGeneration,
      "Room Environment runtime generation changed during Drill E")
  }
  return {
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
  }
}

function tabTarget(action) {
  // In the current runtime, agent Browser Actions entering this ledger are
  // mutations; browser status/find observations bypass it entirely.
  if (action?.mode !== "browser" || !Array.isArray(action.targets) || action.targets.length !== 1) return null
  const target = action.targets[0]
  return target?.kind === "browser_tab" && typeof target.id === "string" && target.id
    ? target.id
    : null
}

function actorKinds(environment) {
  return new Map(environment.actors
    .filter((actor) => actor && typeof actor.actor_id === "string")
    .map((actor) => [actor.actor_id, actor.kind]))
}

function isAgentAction(action, actors, generation) {
  return action
    && action.runtime_generation === generation
    && actors.get(action.actor_id) === "agent"
    && tabTarget(action) !== null
}

function validActionTiming(action) {
  if (!action || typeof action.action_id !== "string" || !action.action_id
    || !safeInteger(action.sequence) || action.sequence < 1
    || typeof action.actor_id !== "string" || !action.actor_id
    || !safeInteger(action.runtime_generation)
    || !["browser", "computer"].includes(action.mode)
    || !Array.isArray(action.targets)
    || typeof action.cancellation_requested !== "boolean"
    || !safeInteger(action.submitted_at_ms)
    || !["queued", "running", "completed", "failed", "cancelled"].includes(action.state)) return false
  if (action.started_at_ms !== null && !safeInteger(action.started_at_ms)) return false
  if (action.finished_at_ms !== null && !safeInteger(action.finished_at_ms)) return false
  if (action.started_at_ms !== null && action.started_at_ms < action.submitted_at_ms) return false
  if (action.finished_at_ms !== null
    && (action.started_at_ms !== null && action.finished_at_ms < action.started_at_ms
      || action.finished_at_ms < action.submitted_at_ms)) return false
  if (action.state === "queued" && (action.started_at_ms !== null || action.finished_at_ms !== null)) return false
  if (action.state === "running" && (action.started_at_ms === null || action.finished_at_ms !== null)) return false
  if (["completed", "failed", "cancelled"].includes(action.state) && action.finished_at_ms === null) return false
  return true
}

function actionOutcome(action) {
  return action?.outcome && typeof action.outcome === "object" ? action.outcome : null
}

function cancelledByTakeover(action) {
  return action?.state === "cancelled"
    && actionOutcome(action)?.status === "cancelled"
    && actionOutcome(action)?.reason === "human_takeover"
}

function matchingTarget(target, tabId) {
  return target?.kind === "browser_tab" && target.id === tabId
}

function sameActionIdentity(projected, recorded) {
  return projected.action_id === recorded.action_id
    && projected.sequence === recorded.sequence
    && projected.actor_id === recorded.actor_id
    && projected.runtime_generation === recorded.runtime_generation
    && projected.mode === recorded.mode
    && projected.submitted_at_ms === recorded.submitted_at_ms
    && JSON.stringify(projected.targets) === JSON.stringify(recorded.targets)
}

function findQueuePair(snapshots, finalEnvironment, actionsById) {
  const finalTabIds = new Set(finalEnvironment.tabs.map((tab) => tab.tab_id))
  for (const sample of snapshots) {
    const environment = sample.environment
    const actors = actorKinds(environment)
    const running = environment.actions.filter((action) =>
      action?.state === "running" && isAgentAction(action, actors, environment.runtime_generation)
      && safeInteger(action.started_at_ms)
      && environment.tabs.some((tab) => tab.tab_id === tabTarget(action)))
    const queued = environment.actions.filter((action) =>
      action?.state === "queued" && isAgentAction(action, actors, environment.runtime_generation)
      && action.started_at_ms == null
      && environment.tabs.some((tab) => tab.tab_id === tabTarget(action)))
    for (const first of running) {
      const firstTab = tabTarget(first)
      if (!finalTabIds.has(firstTab)) continue
      for (const second of queued) {
        const tabId = tabTarget(second)
        if (!tabId || tabId !== firstTab || first.actor_id === second.actor_id
          || second.submitted_at_ms < first.started_at_ms) continue
        const recordedFirst = actionsById.get(first.action_id)
        const recordedSecond = actionsById.get(second.action_id)
        if (!recordedFirst || !recordedSecond
          || !validActionTiming(recordedFirst) || !validActionTiming(recordedSecond)
          || !sameActionIdentity(first, recordedFirst) || !sameActionIdentity(second, recordedSecond)
          || recordedFirst.started_at_ms == null || recordedFirst.finished_at_ms == null
          || recordedSecond.submitted_at_ms >= recordedFirst.finished_at_ms) continue
        const secondWasSerialized = recordedSecond.started_at_ms == null
          ? recordedSecond.state === "cancelled" && cancelledByTakeover(recordedSecond)
          : recordedSecond.started_at_ms >= recordedFirst.finished_at_ms
        if (!secondWasSerialized) continue
        return { sample, first, second, firstTab, recordedFirst, recordedSecond }
      }
    }
  }
  return null
}

function findIndependentTabWork(pair, snapshots, actionsById) {
  if (!pair) return null
  const pairActors = new Set([pair.first.actor_id, pair.second.actor_id])
  for (const sample of snapshots) {
    const environment = sample.environment
    const actors = actorKinds(environment)
    const running = environment.actions.filter((action) =>
      action?.state === "running" && isAgentAction(action, actors, environment.runtime_generation)
      && safeInteger(action.started_at_ms)
      && environment.tabs.some((tab) => tab.tab_id === tabTarget(action)))
    for (const other of running) {
      const otherTab = tabTarget(other)
      if (!otherTab || otherTab === pair.firstTab || pairActors.has(other.actor_id)) continue
      const recordedOther = actionsById.get(other.action_id)
      if (!recordedOther || !validActionTiming(recordedOther)
        || !sameActionIdentity(other, recordedOther)
        || recordedOther.started_at_ms == null || recordedOther.finished_at_ms == null) continue
      const overlapStart = Math.max(pair.recordedFirst.started_at_ms, recordedOther.started_at_ms)
      const overlapEnd = Math.min(pair.recordedFirst.finished_at_ms, recordedOther.finished_at_ms)
      if (overlapStart >= overlapEnd) continue
      return { sample, other, otherTab, recordedOther }
    }
  }
  return null
}

function findTakeover(pair, snapshots, finalEnvironment) {
  if (!pair) return null
  const finalActors = actorKinds(finalEnvironment)
  const forFinalTarget = finalEnvironment.input_ownership.find((ownership) =>
    matchingTarget(ownership?.target, pair.firstTab)
    && finalActors.get(ownership.actor_id) === "human")
  if (!forFinalTarget) return null
  const pending = snapshots.flatMap((sample) => {
    const environment = sample.environment
    const actors = actorKinds(environment)
    return environment.pending_input_takeovers.filter((takeover) =>
      matchingTarget(takeover?.target, pair.firstTab)
      && typeof takeover.human_actor_id === "string"
      && actors.get(takeover.human_actor_id) === "human"
      && Array.isArray(takeover.blocking_action_ids)
      && takeover.blocking_action_ids.includes(pair.first.action_id)
      && environment.actions.some((action) =>
        action.action_id === pair.first.action_id && action.cancellation_requested === true))
      .map((takeover) => ({ sample, takeover }))
  })
  if (!pending.length || pending.every(({ takeover }) => takeover.human_actor_id !== forFinalTarget.actor_id)) return null
  if (!cancelledByTakeover(pair.recordedSecond)) return null
  return {
    pending: pending.at(-1),
    humanActorId: forFinalTarget.actor_id,
    cancelledActionId: pair.recordedSecond.action_id,
  }
}

function historyIndex(actions, finalEnvironment) {
  assert.ok(Array.isArray(actions), "kernel Room Action history is missing")
  assert.ok(actions.length <= maxHistoryActions, "Room Action history exceeds the Drill E bound")
  const knownActors = actorKinds(finalEnvironment)
  const ids = new Set()
  const sequences = new Set()
  const byId = new Map()
  for (const action of actions) {
    assert.ok(action && typeof action === "object" && validActionTiming(action),
      "kernel returned a malformed Room Action history entry")
    assert.ok(typeof action.actor_id === "string" && knownActors.has(action.actor_id),
      "Room Action history Actor is absent from the final kernel snapshot")
    assert.ok(typeof action.mode === "string" && Array.isArray(action.targets),
      "kernel Room Action omitted its mode or targets")
    assert.ok(!ids.has(action.action_id) && !sequences.has(action.sequence),
      "kernel Room Action history contains a duplicate identity or sequence")
    ids.add(action.action_id)
    sequences.add(action.sequence)
    byId.set(action.action_id, action)
  }
  return byId
}

export function verifyDrillE({ sessionId, snapshots, finalEnvironment, actions, baselineActionSequence }) {
  assert.ok(typeof sessionId === "string" && sessionId, "Drill E requires a Room session id")
  assert.ok(Array.isArray(snapshots) && snapshots.length > 0, "Drill E requires observed kernel snapshots")
  assert.ok(safeInteger(baselineActionSequence), "Drill E requires a kernel Action history baseline")
  const identity = assertEnvironment(finalEnvironment, sessionId)
  let previousEventCursor = -1
  for (const sample of snapshots) {
    assert.ok(sample && safeInteger(sample.observed_at_ms), "snapshot observation timestamp is invalid")
    const snapshotIdentity = assertEnvironment(sample.environment, sessionId, identity)
    assert.equal(snapshotIdentity.environmentId, identity.environmentId)
    assert.ok(sample.environment.event_cursor >= previousEventCursor,
      "kernel Environment event cursor moved backwards")
    previousEventCursor = sample.environment.event_cursor
  }
  const postBaselineActions = actions.filter((action) =>
    action.sequence > baselineActionSequence
    && action.runtime_generation === identity.runtimeGeneration)
  const actionsById = historyIndex(postBaselineActions, finalEnvironment)
  const serializedPair = findQueuePair(snapshots, finalEnvironment, actionsById)
  const independent = findIndependentTabWork(serializedPair, snapshots, actionsById)
  const takeover = findTakeover(serializedPair, snapshots, finalEnvironment)

  const checks = {
    twoAgentTabReads: {
      status: "unproven",
      reason: "The current public Tab projection has no reader identity or read timestamp, and browser status/find observations are not recorded in Room Action history.",
      evidence: [],
    },
    sameTabSerialization: serializedPair ? {
      status: "passed",
      tabId: serializedPair.firstTab,
      firstActionId: serializedPair.first.action_id,
      queuedActionId: serializedPair.second.action_id,
      firstIntervalMs: [serializedPair.recordedFirst.started_at_ms, serializedPair.recordedFirst.finished_at_ms],
      queuedSubmittedAtMs: serializedPair.recordedSecond.submitted_at_ms,
      queuedStartedAtMs: serializedPair.recordedSecond.started_at_ms,
    } : {
      status: "incomplete",
      reason: "No observed kernel snapshot showed one agent mutation running while a second agent mutation on the same tab was queued with terminal history proving non-overlap.",
    },
    independentTabConcurrency: independent ? {
      status: "passed",
      firstActionId: serializedPair.first.action_id,
      secondTabActionId: independent.other.action_id,
      firstTabId: serializedPair.firstTab,
      secondTabId: independent.otherTab,
      overlappingIntervalMs: [
        Math.max(serializedPair.recordedFirst.started_at_ms, independent.recordedOther.started_at_ms),
        Math.min(serializedPair.recordedFirst.finished_at_ms, independent.recordedOther.finished_at_ms),
      ],
    } : {
      status: "incomplete",
      reason: "No overlapping kernel-recorded intervals showed a third agent working on a different Room tab while the same-tab mutation was active.",
    },
    humanTakeover: takeover ? {
      status: "passed",
      tabId: serializedPair.firstTab,
      humanActorId: takeover.humanActorId,
      blockingActionId: serializedPair.first.action_id,
      cancelledActionId: takeover.cancelledActionId,
      pendingObservedAtMs: takeover.pending.sample.observed_at_ms,
    } : {
      status: "incomplete",
      reason: "No observed pending human takeover with kernel-requested cancellation, takeover-attributed terminal cancellation, and final human ownership was found.",
    },
  }
  const status = Object.values(checks).every((check) => check.status === "passed") ? "passed" : "incomplete"
  return {
    schema: "chariox.browser_computer.drill_e.kernel_evidence.v1",
    status,
    sessionId,
    environmentId: identity.environmentId,
    runtimeGeneration: identity.runtimeGeneration,
    baselineActionSequence,
    observedSnapshots: snapshots.length,
    historyActionCount: actions.length,
    postBaselineActionCount: postBaselineActions.length,
    checks,
  }
}

export async function readDrillEActionHistory(client, requests, sessionId) {
  const actions = []
  const actionIds = new Set()
  let beforeSequence = null
  while (true) {
    const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(
      sessionId,
      beforeSequence,
      pageSize,
    ))
    const page = unwrap(response, "RoomEnvironmentActionHistoryListed")?.page
    assert.ok(page && Array.isArray(page.actions) && page.actions.length <= pageSize,
      "kernel returned a malformed Room Action history page")
    for (const action of page.actions) {
      assert.ok(validActionTiming(action), "kernel returned a malformed Room Action history entry")
      assert.ok(beforeSequence === null || action.sequence < beforeSequence,
        "Room Action history cursor did not move backwards")
      assert.ok(!actionIds.has(action.action_id), "Room Action history repeated an action id")
      actionIds.add(action.action_id)
      actions.push(action)
      assert.ok(actions.length <= maxHistoryActions, "Room Action history exceeds the Drill E bound")
    }
    const next = page.next_before_sequence
    if (next == null) return actions
    assert.ok(page.actions.length > 0 && safeInteger(next) && next > 0
      && (beforeSequence === null || next < beforeSequence),
    "Room Action history cursor did not advance")
    beforeSequence = next
  }
}

export async function captureDrillEFromClient({
  client,
  requests,
  sessionId,
  observeMs = defaultObserveMs,
  pollMs = 1_000,
  now = Date.now,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  assert.ok(client && typeof client.send === "function", "Drill E requires a connected LocalIpcClient")
  assert.ok(requests && typeof requests.getRoomEnvironmentStateRequest === "function"
    && typeof requests.listRoomEnvironmentActionHistoryRequest === "function",
  "Drill E requires the public Room Environment request builders")
  assert.ok(typeof sessionId === "string" && sessionId, "Drill E requires a Room session id")
  assert.ok(Number.isSafeInteger(observeMs) && observeMs >= 0 && observeMs <= maxObserveMs,
    "Drill E observation duration exceeds its bound")
  assert.ok(Number.isSafeInteger(pollMs) && pollMs > 0 && pollMs <= maxPollMs,
    "Drill E poll interval is invalid")

  const baselineResponse = await client.send(requests.getRoomEnvironmentStateRequest(sessionId))
  const baselineEnvironment = unwrap(baselineResponse, "RoomEnvironmentState")?.environment
  const identity = assertEnvironment(baselineEnvironment, sessionId)
  const baselineSnapshot = { observed_at_ms: now(), environment: baselineEnvironment }
  const baselineActions = await readDrillEActionHistory(client, requests, sessionId)
  const baselineActionSequence = Math.max(
    0,
    ...baselineActions
      .filter((action) => action.runtime_generation === identity.runtimeGeneration)
      .map((action) => action.sequence),
    ...baselineEnvironment.actions
      .filter((action) => action.runtime_generation === identity.runtimeGeneration)
      .map((action) => action.sequence),
  )
  const snapshots = [baselineSnapshot]
  const deadline = now() + observeMs
  do {
    const response = await client.send(requests.getRoomEnvironmentStateRequest(sessionId))
    const environment = unwrap(response, "RoomEnvironmentState")?.environment
    assertEnvironment(environment, sessionId, identity)
    snapshots.push({ observed_at_ms: now(), environment })
    if (snapshots.length > Math.ceil(maxObserveMs / minPollMs) + 2) {
      throw new Error("Drill E snapshot count exceeded its bound")
    }
    if (now() >= deadline) break
    await sleep(Math.min(pollMs, Math.max(1, deadline - now())))
  } while (true)

  const actions = await readDrillEActionHistory(client, requests, sessionId)
  const finalResponse = await client.send(requests.getRoomEnvironmentStateRequest(sessionId))
  const finalEnvironment = unwrap(finalResponse, "RoomEnvironmentState")?.environment
  assertEnvironment(finalEnvironment, sessionId, identity)
  const finalObservedAtMs = now()
  snapshots.push({ observed_at_ms: finalObservedAtMs, environment: finalEnvironment })
  const report = verifyDrillE({
    sessionId,
    snapshots,
    finalEnvironment,
    actions,
    baselineActionSequence,
  })
  return { snapshots, finalEnvironment, actions, report }
}

function redactedAction(action) {
  return {
    actionId: action.action_id,
    sequence: action.sequence,
    actorId: action.actor_id,
    runtimeGeneration: action.runtime_generation,
    mode: action.mode,
    targets: action.targets,
    state: action.state,
    cancellationRequested: action.cancellation_requested,
    submittedAtMs: action.submitted_at_ms,
    startedAtMs: action.started_at_ms,
    finishedAtMs: action.finished_at_ms,
    outcome: action.outcome,
  }
}

function redactedSnapshot(sample) {
  const environment = sample.environment
  return {
    observedAtMs: sample.observed_at_ms,
    sessionId: environment.session_id,
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
    lifecycle: environment.lifecycle,
    eventCursor: environment.event_cursor,
    actors: environment.actors.map(({ actor_id, kind, presence }) => ({ actorId: actor_id, kind, presence })),
    tabIds: environment.tabs.map(({ tab_id }) => tab_id),
    actions: environment.actions.map(redactedAction),
    pendingInputTakeovers: environment.pending_input_takeovers,
    inputOwnership: environment.input_ownership,
  }
}

function defaultOutputPath() {
  const stamp = `${new Date().toISOString().replace(/[:.]/g, "-")}-${process.pid}`
  return path.join(os.homedir(), ".chariox", "dev", "browser-computer-use", `drill-e-${stamp}.json`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log(helpText())
    return
  }
  validateOptions(options)
  const outputPath = options.output ?? defaultOutputPath()
  assert.ok(outputPath !== repoRoot && !outputPath.startsWith(`${repoRoot}${path.sep}`),
    "Drill E report must be outside the repository")

  const ipcUrl = pathToFileURL(path.resolve(repoRoot, "packages/kernel-client/dist/ipc.js")).href
  const requestsUrl = pathToFileURL(path.resolve(repoRoot, "packages/kernel-client/dist/ipc-requests.js")).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  const client = new LocalIpcClient(options.kernelUrl)
  try {
    console.log(`[drill-e] attached read-only to Room ${options.sessionId}; observing for ${options.observeMs} ms`)
    const capture = await captureDrillEFromClient({
      client,
      requests,
      sessionId: options.sessionId,
      observeMs: options.observeMs,
      pollMs: options.pollMs,
    })
    const evidence = {
      ...capture.report,
      capturedAt: new Date().toISOString(),
      source: "LocalIpcClient RoomEnvironmentState snapshots and paged RoomEnvironmentActionHistory",
      snapshots: capture.snapshots.map(redactedSnapshot),
      actionHistory: capture.actions
        .filter((action) => action.sequence > capture.report.baselineActionSequence
          && action.runtime_generation === capture.report.runtimeGeneration)
        .map(redactedAction),
    }
    await mkdir(path.dirname(outputPath), { recursive: true, mode: 0o700 })
    await writeFile(outputPath, `${JSON.stringify(evidence, null, 2)}\n`, { encoding: "utf8", mode: 0o600, flag: "wx" })
    console.log(`[drill-e] ${capture.report.status}; report: ${outputPath}`)
    for (const [name, check] of Object.entries(capture.report.checks)) {
      console.log(`[drill-e] ${name}: ${check.status}${check.reason ? ` — ${check.reason}` : ""}`)
    }
    if (capture.report.status !== "passed") process.exitCode = 2
  } finally {
    await client.close?.().catch(() => {})
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`[drill-e] failed closed: ${error?.stack ?? error}`)
    process.exitCode = 1
  })
}

import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { readDrillEActionHistory, verifyDrillE } from "../live-browser-computer-drill-e.mjs"

export const drillEScenarioCleanupBudgetMs = 5_000
export const drillEScenarioDetachReserveMs = 1_000
const cleanupTimeoutMs = drillEScenarioCleanupBudgetMs
const detachReserveMs = drillEScenarioDetachReserveMs
const cleanupPollMs = 100
const minPollMs = 250
const readKinds = new Set(["browser_status", "browser_find"])
const reloadKind = "browser_history_reload"

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && Object.hasOwn(response, variant),
    "kernel response omitted " + variant)
  return response[variant]
}

function unwrapSession(response) {
  for (const variant of ["SessionState", "SessionStateLoaded"]) {
    if (response && typeof response === "object" && Object.hasOwn(response, variant)) {
      return response[variant]?.session
    }
  }
  throw new Error("kernel response omitted session state")
}

function assertId(value, name) {
  assert.ok(typeof value === "string" && value.length > 0 && value.length <= 256
    && value === value.trim() && !/[\u0000-\u001f\u007f]/u.test(value),
  name + " must be a non-empty bounded identity")
}

function tabTarget(action) {
  if (!Array.isArray(action?.targets)) return null
  const target = action.targets.find((item) => item?.kind === "browser_tab")
  return typeof target?.id === "string" ? target.id : null
}

function assertPreconditions(session, environment, options) {
  assert.ok(session && session.id === options.sessionId && Array.isArray(session.agents),
    "kernel returned a different or malformed Room session")
  const sessionAgents = new Set(session.agents.map((agent) => agent?.id).filter((id) => typeof id === "string"))
  for (const agentId of [options.agentA, options.agentB, options.agentC]) {
    assert.ok(sessionAgents.has(agentId), "Room does not contain requested agent " + agentId)
  }
  assert.ok(environment && environment.session_id === options.sessionId
    && environment.lifecycle === "ready"
    && Number.isSafeInteger(environment.runtime_generation)
    && Number.isSafeInteger(environment.event_cursor)
    && typeof environment.environment_id === "string" && environment.environment_id
    && environment.viewport && Number.isSafeInteger(environment.viewport.revision)
    && Array.isArray(environment.health) && Array.isArray(environment.pointers)
    && Array.isArray(environment.actors) && Array.isArray(environment.tabs)
    && Array.isArray(environment.actions) && Array.isArray(environment.pending_input_takeovers)
    && Array.isArray(environment.input_ownership),
  "Room Environment is not a ready, well-formed kernel snapshot")
  const actorById = new Map(environment.actors.map((actor) => [actor?.actor_id, actor]))
  for (const agentId of [options.agentA, options.agentB, options.agentC]) {
    const actor = actorById.get("agent:" + agentId)
    assert.ok(actor?.kind === "agent" && actor.presence === "present",
      "requested agent " + agentId + " is not present in the Room Environment")
  }
  const tabs = new Map(environment.tabs.map((tab) => [tab?.tab_id, tab]))
  assert.ok(tabs.has(options.sameTabId) && tabs.has(options.otherTabId),
    "both requested browser tabs must exist in the Room Environment")
  assert.equal(environment.focused_tab_id, options.sameTabId,
    "--same-tab must already be the focused tab so the scenario does not change operator focus")
  assert.ok(!environment.pending_input_takeovers.some((takeover) =>
    takeover?.target?.kind === "browser_tab"
      && [options.sameTabId, options.otherTabId].includes(takeover.target.id)),
  "requested tabs already have a pending input takeover")
  assert.ok(!environment.actions.some((action) =>
    ["running", "queued"].includes(action?.state)
      && [options.sameTabId, options.otherTabId].includes(tabTarget(action))),
  "requested tabs already have active Room actions")
  return {
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
  }
}

function readPrompt() {
  return [
    "For this Drill E phase, use only slice_browser_status and slice_browser_find on the currently focused tab.",
    "Perform three read-only rounds: call slice_browser_status, then slice_browser_find with query \"Drill E probe\" and kind \"any\".",
    "Do not activate, reload, navigate, click, type, or use any mutation tool. After the three rounds, return a short summary.",
  ].join(" ")
}

function historyPrompt(tabId, purpose) {
  return [
    "For Drill E " + purpose + ", call slice_browser_history exactly once with tab_id "
      + JSON.stringify(tabId) + " and action \"reload\".",
    "Do not use another browser tool or act on any other tab; after the call returns, stop and report that it finished.",
  ].join(" ")
}

function boundedClient(client, deadline, now, message = "Drill E request exceeded its bounded deadline") {
  return {
    async send(request) {
      const remainingMs = deadline - now()
      assert.ok(remainingMs > 0, "Drill E request deadline expired")
      let timer
      const timeout = new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new Error(message)), remainingMs)
      })
      return Promise.race([Promise.resolve().then(() => client.send(request)), timeout])
        .finally(() => clearTimeout(timer))
    },
  }
}

async function submitPrompt(client, requests, sessionId, attachmentId, agentId, prompt, purpose, ledger) {
  const owned = {
    agentId,
    purpose,
    promptId: null,
    admission: "submitting",
    cleanupStatus: "unresolved",
  }
  ledger.push(owned)
  try {
    const body = unwrap(await client.send(requests.submitPromptRequest(
      sessionId, attachmentId, agentId, prompt, [],
    )), "PromptSubmitted")
    const outcome = body?.outcome
    const promptRecord = outcome?.Started?.prompt ?? outcome?.Queued?.prompt
    assert.ok(promptRecord && promptRecord.target_agent_id === agentId,
      "kernel did not acknowledge the prompt for its exact requested agent")
    assertId(promptRecord.id, "kernel prompt id")
    assert.equal(promptRecord.source_attachment_id, attachmentId,
      "kernel prompt was not submitted by the Drill E attachment")
    owned.promptId = promptRecord.id
    owned.admission = outcome.Started ? "started" : "queued"
    return owned
  } catch (error) {
    if (owned.promptId === null) owned.admission = "acknowledgment_unknown"
    throw error
  }
}

async function submitTogether(client, requests, sessionId, attachmentId, entries, ledger) {
  const results = await Promise.allSettled(entries.map(([agentId, prompt, purpose]) =>
    submitPrompt(client, requests, sessionId, attachmentId, agentId, prompt, purpose, ledger)))
  const failed = results.find((result) => result.status === "rejected")
  if (failed) throw failed.reason
}

function readOverlap(environment, options) {
  const active = environment.actions.filter((action) => action?.state === "running")
  const a = active.find((action) => action.actor_id === "agent:" + options.agentA
    && tabTarget(action) === options.sameTabId && readKinds.has(action.kind))
  const b = active.find((action) => action.actor_id === "agent:" + options.agentB
    && tabTarget(action) === options.sameTabId && readKinds.has(action.kind))
  const c = active.find((action) => action.actor_id === "agent:" + options.agentC
    && tabTarget(action) === options.otherTabId && action.kind === reloadKind)
  return a && b && c ? { a, b, c } : null
}

function runningMutation(environment, agentId, tabId) {
  return environment.actions.find((action) => action?.state === "running"
    && action.actor_id === "agent:" + agentId && tabTarget(action) === tabId
    && action.kind === reloadKind) ?? null
}

function queuePair(environment, options) {
  const first = runningMutation(environment, options.agentA, options.sameTabId)
  const second = environment.actions.find((action) => action?.state === "queued"
    && action.actor_id === "agent:" + options.agentB
    && tabTarget(action) === options.sameTabId && action.kind === reloadKind)
  return first && second && second.submitted_at_ms >= first.started_at_ms
    ? { first, second }
    : null
}

function humanOwnsTab(environment, tabId) {
  const actors = new Map(environment.actors.map((actor) => [actor?.actor_id, actor?.kind]))
  return environment.input_ownership.some((ownership) =>
    ownership?.target?.kind === "browser_tab" && ownership.target.id === tabId
      && actors.get(ownership.actor_id) === "human")
}

function maxSnapshotCount(timeoutMs) {
  return Math.ceil(timeoutMs / minPollMs) + 5
}

function relevantSnapshotKinds(environment, options) {
  const kinds = []
  const reads = readOverlap(environment, options)
  if (reads) kinds.push("reads:" + reads.a.action_id + ":" + reads.b.action_id + ":" + reads.c.action_id)
  const pair = queuePair(environment, options)
  if (pair) kinds.push("queue:" + pair.first.action_id + ":" + pair.second.action_id)
  const first = runningMutation(environment, options.agentA, options.sameTabId)
  const third = environment.actions.find((action) => action?.state === "running"
    && action.actor_id === "agent:" + options.agentC
    && tabTarget(action) === options.otherTabId && action.kind === reloadKind)
  if (first && third) kinds.push("independent:" + first.action_id + ":" + third.action_id)
  if (environment.pending_input_takeovers.some((takeover) =>
    takeover?.target?.kind === "browser_tab" && takeover.target.id === options.sameTabId)) {
    kinds.push("takeover")
  }
  return kinds
}

function targetOwner(environment, tabId) {
  return environment?.input_ownership?.find((ownership) =>
    ownership?.target?.kind === "browser_tab" && ownership.target.id === tabId) ?? null
}

function promptInventory(session) {
  const byId = new Map()
  const add = (prompt, location) => {
    if (!prompt || typeof prompt.id !== "string") return
    const previous = byId.get(prompt.id)
    if (!previous || location === "active") byId.set(prompt.id, { prompt, location })
  }
  add(session.active_prompt, "active")
  for (const prompt of session.queued_prompts ?? []) add(prompt, "queued")
  for (const state of Object.values(session.prompt_states ?? {})) {
    add(state?.active_prompt, "active")
    for (const prompt of state?.queued_prompts ?? []) add(prompt, "queued")
  }
  return byId
}

function hasPromptInventory(session) {
  return Boolean(session && (
    session.prompt_states && typeof session.prompt_states === "object"
  ))
}

function cleanupFailure(cleanup, code, fields = {}) {
  const key = code + ":" + JSON.stringify(fields)
  if (cleanup._failureKeys.has(key)) return
  cleanup._failureKeys.add(key)
  cleanup.failures.push({ code, ...fields })
}

async function cleanupOwnedPrompts({
  requests,
  sessionId,
  attachmentId,
  prompts,
  cleanupClient,
  workDeadline,
  now,
  sleep,
  cleanup,
}) {
  if (!prompts.length) return
  const cancelAttempted = new Set()
  const unknownRecorded = new Set()
  while (now() < workDeadline) {
    let session
    try {
      session = unwrapSession(await cleanupClient.send(requests.getSessionStateRequest(sessionId)))
    } catch {
      cleanupFailure(cleanup, "owned_prompt_state_unavailable")
      return
    }
    if (!hasPromptInventory(session)) {
      cleanupFailure(cleanup, "owned_prompt_state_projection_missing")
      return
    }
    const inventory = promptInventory(session)
    const stillPending = []
    for (const owned of prompts) {
      if (!owned.promptId) {
        if (!unknownRecorded.has(owned)) {
          cleanupFailure(cleanup, "submitted_prompt_id_unavailable", { agentId: owned.agentId, purpose: owned.purpose })
          unknownRecorded.add(owned)
        }
        owned.cleanupStatus = "identity_unknown"
        continue
      }
      const located = inventory.get(owned.promptId)
      if (!located) {
        owned.cleanupStatus = "settled"
        continue
      }
      const prompt = located.prompt
      if (prompt.target_agent_id !== owned.agentId
        || (prompt.source_attachment_id != null && prompt.source_attachment_id !== attachmentId)) {
        cleanupFailure(cleanup, "owned_prompt_identity_mismatch", {
          agentId: owned.agentId,
          promptId: owned.promptId,
        })
        owned.cleanupStatus = "identity_mismatch"
        continue
      }
      if (located.location === "active") {
        owned.cleanupStatus = "active_cancel_seam_missing"
        stillPending.push(owned)
        continue
      }
      owned.cleanupStatus = cancelAttempted.has(owned.promptId) ? "queued_cancel_unconfirmed" : "queued"
      stillPending.push(owned)
      if (cancelAttempted.has(owned.promptId)) continue
      cancelAttempted.add(owned.promptId)
      try {
        const response = await cleanupClient.send(requests.cancelQueuedPromptRequest(
          sessionId, attachmentId, owned.agentId, owned.promptId,
        ))
        const cancelled = unwrap(response, "QueuedPromptCancelled")
        assert.equal(cancelled?.prompt?.id, owned.promptId,
          "kernel cancelled a different queued prompt")
        assert.equal(cancelled.prompt.target_agent_id, owned.agentId,
          "kernel cancelled a prompt for a different agent")
        owned.cleanupStatus = "queued_cancelled"
      } catch {
        owned.cleanupStatus = "queued_cancel_unconfirmed"
      }
    }
    if (!stillPending.length) return
    const remainingMs = workDeadline - now()
    if (remainingMs <= 0) break
    await sleep(Math.min(cleanupPollMs, remainingMs))
  }
  for (const owned of prompts) {
    if (owned.cleanupStatus === "active_cancel_seam_missing") {
      cleanupFailure(cleanup, "owned_active_prompt_cancel_by_id_unavailable", {
        agentId: owned.agentId,
        promptId: owned.promptId,
      })
    } else if (owned.cleanupStatus === "queued_cancel_unconfirmed"
      || owned.cleanupStatus === "queued_cancelled"
      || owned.cleanupStatus === "queued") {
      cleanupFailure(cleanup, "owned_queued_prompt_not_confirmed_settled", {
        agentId: owned.agentId,
        promptId: owned.promptId,
      })
    }
  }
}

async function cleanupTakeover({
  requests,
  sessionId,
  takeover,
  cleanupClient,
  cleanup,
}) {
  const record = cleanup.takeover
  if (!takeover.requested) {
    record.status = "not_requested"
    return
  }
  record.previousOwnerActorId = takeover.previousOwnerActorId
  record.humanActorId = takeover.humanActorId
  record.blockingActionIds = [...takeover.blockingActionIds]
  record.environmentId = takeover.environmentId
  record.runtimeGeneration = takeover.runtimeGeneration
  let environment
  try {
    environment = unwrap(await cleanupClient.send(
      requests.getRoomEnvironmentStateRequest(sessionId),
    ), "RoomEnvironmentState")?.environment
  } catch {
    record.status = "state_unavailable"
    cleanupFailure(cleanup, "takeover_state_unavailable", { targetTabId: takeover.tabId })
    return
  }
  if (environment?.environment_id !== takeover.environmentId
    || environment?.runtime_generation !== takeover.runtimeGeneration) {
    record.status = "environment_changed"
    cleanupFailure(cleanup, "takeover_environment_changed", { targetTabId: takeover.tabId })
    return
  }
  const pending = environment?.pending_input_takeovers?.find((item) =>
    item?.target?.kind === "browser_tab" && item.target.id === takeover.tabId)
  const owner = targetOwner(environment, takeover.tabId)
  if (pending) {
    record.pendingHumanActorId = pending.human_actor_id ?? null
    record.status = "pending_cancel_seam_missing"
    cleanupFailure(cleanup, "pending_takeover_cannot_be_retracted", { targetTabId: takeover.tabId })
    return
  }
  if (!takeover.responseObserved) {
    record.status = "request_result_unknown"
    cleanupFailure(cleanup, "takeover_request_result_unknown", { targetTabId: takeover.tabId })
    return
  }
  if (!takeover.humanActorId) {
    record.status = "actor_identity_unknown"
    cleanupFailure(cleanup, "takeover_actor_identity_unavailable", { targetTabId: takeover.tabId })
    return
  }
  if (takeover.previousOwnerActorId === takeover.humanActorId) {
    record.status = "preexisting_owner_preserved"
    return
  }
  if (owner?.actor_id !== takeover.humanActorId) {
    record.status = "no_longer_owned_by_run"
    return
  }
  try {
    const released = unwrap(await cleanupClient.send(requests.releaseRoomEnvironmentInputRequest(
      sessionId,
      { kind: "browser_tab", id: takeover.tabId },
    )), "RoomEnvironmentInputReleased")
    assert.equal(released?.environment?.environment_id, takeover.environmentId,
      "kernel released input in a different Environment")
    assert.equal(released.environment.runtime_generation, takeover.runtimeGeneration,
      "kernel released input in a different Environment runtime")
    const remainingOwner = targetOwner(released?.environment, takeover.tabId)
    assert.notEqual(remainingOwner?.actor_id, takeover.humanActorId,
      "kernel retained the Drill E human input owner after release")
    record.status = "released"
  } catch {
    record.status = "release_failed"
    cleanupFailure(cleanup, "owned_takeover_release_failed", { targetTabId: takeover.tabId })
  }
}

async function cleanupDrillState({
  client,
  requests,
  sessionId,
  attachmentId,
  attachmentAttempted,
  environmentIdentity,
  prompts,
  takeover,
  now,
  sleep,
}) {
  const startedAt = now()
  const deadline = startedAt + cleanupTimeoutMs
  const workDeadline = deadline - detachReserveMs
  const cleanupClient = boundedClient(client, workDeadline, now, "Drill E cleanup work exceeded its independent budget")
  const detachClient = boundedClient(client, deadline, now, "Drill E detach exceeded its independent budget")
  const cleanup = {
    status: "passed",
    stage: attachmentId === null && attachmentAttempted ? "attachment_identity_unavailable" : "starting",
    budgetMs: cleanupTimeoutMs,
    detachReserveMs,
    attachmentId,
    environmentId: environmentIdentity?.environmentId ?? null,
    runtimeGeneration: environmentIdentity?.runtimeGeneration ?? null,
    submittedPrompts: prompts.map(({ agentId, promptId, purpose, admission }) => ({
      agentId,
      promptId,
      purpose,
      admission,
      cleanupStatus: "unresolved",
    })),
    takeover: {
      targetTabId: takeover.tabId,
      requested: takeover.requested,
      previousOwnerActorId: null,
      humanActorId: null,
      blockingActionIds: [],
      status: "not_requested",
    },
    detached: !attachmentAttempted,
    failures: [],
    _failureKeys: new Set(),
  }
  const promptRecords = cleanup.submittedPrompts
  if (attachmentId === null && attachmentAttempted) {
    cleanupFailure(cleanup, "session_attachment_identity_unavailable")
  } else if (attachmentId !== null) {
    cleanup.stage = "owned_prompts"
    try {
      await cleanupOwnedPrompts({
        requests,
        sessionId,
        attachmentId,
        prompts,
        cleanupClient,
        workDeadline,
        now,
        sleep,
        cleanup,
      })
    } catch {
      cleanupFailure(cleanup, "owned_prompt_cleanup_failed")
    }
    cleanup.stage = "takeover"
    try {
      await cleanupTakeover({
        requests,
        sessionId,
        takeover,
        cleanupClient,
        cleanup,
      })
    } catch {
      cleanupFailure(cleanup, "owned_takeover_cleanup_failed", { targetTabId: takeover.tabId })
    }
    cleanup.stage = "detach"
    try {
      unwrap(await detachClient.send(requests.detachFromSessionRequest(attachmentId)), "SessionDetached")
      cleanup.detached = true
    } catch {
      cleanup.detached = false
      cleanupFailure(cleanup, "session_detach_failed")
    }
    cleanup.stage = "finished"
  }
  for (let index = 0; index < prompts.length; index += 1) {
    promptRecords[index].cleanupStatus = prompts[index].cleanupStatus
  }
  cleanup.status = cleanup.failures.length === 0 ? "passed" : "failed"
  delete cleanup._failureKeys
  return cleanup
}

function attachCleanup(error, cleanup) {
  if (error && typeof error === "object") {
    Object.defineProperty(error, "cleanup", { value: cleanup, configurable: true })
    return error
  }
  const wrapped = new Error("Drill E failed before producing a report")
  wrapped.cleanup = cleanup
  return wrapped
}

export async function runDrillEScenario({
  client,
  requests,
  options,
  now = Date.now,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
}) {
  assert.ok(options.execute, "scenario execution requires explicit --execute opt-in")
  assert.ok(client && typeof client.send === "function", "Drill E requires LocalIpcClient.send")
  for (const name of [
    "attachToSessionRequest", "detachFromSessionRequest", "getSessionStateRequest",
    "getRoomEnvironmentStateRequest", "listRoomEnvironmentActionHistoryRequest",
    "submitPromptRequest", "requestRoomEnvironmentInputTakeoverRequest",
    "cancelQueuedPromptRequest", "releaseRoomEnvironmentInputRequest",
  ]) assert.ok(typeof requests?.[name] === "function", "missing public request builder " + name)

  let attachmentId = null
  let attachmentAttempted = false
  let environmentIdentity = null
  const startedAt = now()
  const deadline = startedAt + options.timeoutMs
  const bounded = boundedClient(client, deadline, now, "Drill E request exceeded its total timeout")
  const prompts = []
  const takeover = {
    requested: false,
    responseObserved: false,
    tabId: options.sameTabId,
    previousOwnerActorId: null,
    humanActorId: null,
    blockingActionIds: [],
    environmentId: null,
    runtimeGeneration: null,
  }
  let capture = null
  let failure = null
  let failureStage = "attach"
  let stage = "attach"
  let cleanup = null
  try {
    attachmentAttempted = true
    const attached = unwrap(await bounded.send(requests.attachToSessionRequest(
      options.sessionId, "drill-e-" + randomUUID(),
    )), "SessionAttached")
    attachmentId = attached?.attachment?.id
    assertId(attachmentId, "kernel attachment id")
    stage = "session_preflight"
    const session = unwrapSession(await bounded.send(requests.getSessionStateRequest(options.sessionId)))
    stage = "environment_preflight"
    const baselineEnvironment = unwrap(
      await bounded.send(requests.getRoomEnvironmentStateRequest(options.sessionId)),
      "RoomEnvironmentState",
    )?.environment
    const identity = assertPreconditions(session, baselineEnvironment, options)
    environmentIdentity = identity
    const baselineSnapshot = { observed_at_ms: now(), environment: baselineEnvironment }
    stage = "baseline_action_history"
    const baselineActions = await readDrillEActionHistory(bounded, requests, options.sessionId)
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
    const maxObservations = maxSnapshotCount(options.timeoutMs)
    const maxRetainedSnapshots = 40
    const retainedKinds = new Set()
    const retainedKindCounts = new Map()
    let observationCount = 0
    const retain = (environment, { force = false, forcedKind = null } = {}) => {
      const kinds = relevantSnapshotKinds(environment, options)
      if (forcedKind) kinds.push(forcedKind)
      const freshKinds = kinds.filter((kind) => !retainedKinds.has(kind)
        && (kind === "takeover" || (retainedKindCounts.get(kind.split(":", 1)[0]) ?? 0) < 8))
      if (!force && freshKinds.length === 0) return
      assert.ok(snapshots.length < maxRetainedSnapshots, "Drill E retained snapshot count exceeded its bound")
      snapshots.push({ observed_at_ms: now(), environment })
      for (const kind of freshKinds) {
        retainedKinds.add(kind)
        const category = kind.split(":", 1)[0]
        retainedKindCounts.set(category, (retainedKindCounts.get(category) ?? 0) + 1)
      }
    }
    const observe = async () => {
      assert.ok(observationCount < maxObservations, "Drill E observation count exceeded its bound")
      const response = await bounded.send(requests.getRoomEnvironmentStateRequest(options.sessionId))
      const environment = unwrap(response, "RoomEnvironmentState")?.environment
      observationCount += 1
      retain(environment)
      return environment
    }
    const waitFor = async (predicate, stageDeadline) => {
      while (now() < stageDeadline && now() < deadline) {
        const environment = await observe()
        const result = predicate(environment)
        if (result) return result
        const remaining = Math.min(stageDeadline, deadline) - now()
        if (remaining > 0) await sleep(Math.min(options.pollMs, remaining))
      }
      return null
    }

    stage = "read_prompt_submission"
    await submitTogether(bounded, requests, options.sessionId, attachmentId, [
      [options.agentA, readPrompt(), "same-tab reads A"],
      [options.agentB, readPrompt(), "same-tab reads B"],
      [options.agentC, historyPrompt(options.otherTabId, "independent-tab work"), "independent-tab work C"],
    ], prompts)

    stage = "read_overlap_observation"
    await waitFor((environment) => readOverlap(environment, options),
      startedAt + Math.floor(options.timeoutMs * 0.3))

    let firstMutation = null
    if (now() < deadline) {
      stage = "first_mutation_prompt_submission"
      await submitTogether(bounded, requests, options.sessionId, attachmentId, [
        [options.agentA, historyPrompt(options.sameTabId, "first same-tab mutation"), "first mutation A"],
        [options.agentC, historyPrompt(options.otherTabId, "independent-tab concurrency"), "independent mutation C"],
      ], prompts)
      stage = "first_mutation_observation"
      firstMutation = await waitFor((environment) =>
        runningMutation(environment, options.agentA, options.sameTabId),
      startedAt + Math.floor(options.timeoutMs * 0.6))
    }

    if (firstMutation && now() < deadline) {
      stage = "second_mutation_prompt_submission"
      await submitPrompt(bounded, requests, options.sessionId, attachmentId, options.agentB,
        historyPrompt(options.sameTabId, "second same-tab mutation"), "second mutation B", prompts)
      stage = "same_tab_queue_observation"
      const pair = now() < deadline
        ? await waitFor((environment) => queuePair(environment, options),
          startedAt + Math.floor(options.timeoutMs * 0.8))
        : null
      if (pair) {
        stage = "human_takeover_request"
        takeover.previousOwnerActorId = targetOwner(baselineEnvironment, options.sameTabId)?.actor_id ?? null
        takeover.environmentId = identity.environmentId
        takeover.runtimeGeneration = identity.runtimeGeneration
        takeover.requested = true
        const takeoverResponse = unwrap(await bounded.send(requests.requestRoomEnvironmentInputTakeoverRequest(
          options.sessionId,
          { kind: "browser_tab", id: options.sameTabId },
        )), "RoomEnvironmentTakeoverUpdated")
        takeover.responseObserved = true
        const outcome = takeoverResponse?.outcome
        const takeoverEnvironment = takeoverResponse?.environment
        const pending = takeoverEnvironment?.pending_input_takeovers?.find((item) =>
          item?.target?.kind === "browser_tab" && item.target.id === options.sameTabId)
        const owner = targetOwner(takeoverEnvironment, options.sameTabId)
        takeover.humanActorId = pending?.human_actor_id
          ?? (takeoverEnvironment?.actors?.some((actor) =>
            actor?.actor_id === owner?.actor_id && actor.kind === "human") ? owner?.actor_id : null)
          ?? null
        takeover.blockingActionIds = Array.isArray(outcome?.action_ids) ? [...outcome.action_ids] : []
        assert.ok(outcome?.state === "cancellation_required"
          && takeover.blockingActionIds.includes(pair.first.action_id),
        "kernel takeover response did not bind cancellation to the observed running mutation")
        if (takeoverEnvironment) retain(takeoverEnvironment, { force: true, forcedKind: "takeover" })
        stage = "human_takeover_observation"
        await waitFor((environment) => humanOwnsTab(environment, options.sameTabId), deadline)
      }
    }

    stage = "final_environment_observation"
    const finalResponse = await bounded.send(requests.getRoomEnvironmentStateRequest(options.sessionId))
    const finalEnvironment = unwrap(finalResponse, "RoomEnvironmentState")?.environment
    retain(finalEnvironment, { force: true, forcedKind: "final" })
    stage = "action_history"
    const actions = await readDrillEActionHistory(bounded, requests, options.sessionId)
    stage = "evidence_verification"
    const report = verifyDrillE({
      sessionId: options.sessionId,
      snapshots,
      finalEnvironment,
      actions,
      baselineActionSequence,
    })
    capture = {
      snapshots,
      finalEnvironment,
      actions,
      report,
      submittedPrompts: prompts,
      kernelStatePollCount: observationCount + 1,
    }
  } catch (error) {
    failure = error
    failureStage = stage
  } finally {
    stage = "cleanup"
    try {
      cleanup = await cleanupDrillState({
        client,
        requests,
        sessionId: options.sessionId,
        attachmentId,
        attachmentAttempted,
        environmentIdentity,
        prompts,
        takeover,
        now,
        sleep,
      })
    } catch {
      cleanup = {
        status: "failed",
        stage: "cleanup_runner",
        budgetMs: cleanupTimeoutMs,
        detached: false,
        submittedPrompts: prompts.map(({ agentId, promptId, purpose, admission }) => ({
          agentId, promptId, purpose, admission, cleanupStatus: "unknown",
        })),
        takeover: { requested: takeover.requested, targetTabId: takeover.tabId, status: "unknown" },
        failures: [{ code: "cleanup_runner_failed" }],
      }
    }
  }
  if (capture) {
    capture.cleanup = cleanup
    if (cleanup.status !== "passed") capture.report.status = "failed"
    return capture
  }
  if (failure) {
    Object.defineProperty(failure, "drillEStage", { value: failureStage, configurable: true })
    throw attachCleanup(failure, cleanup)
  }
  const error = new Error("Drill E ended without a capture")
  Object.defineProperty(error, "drillEStage", { value: stage, configurable: true })
  throw attachCleanup(error, cleanup)
}

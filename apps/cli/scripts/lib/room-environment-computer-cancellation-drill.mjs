import assert from "node:assert/strict"

function computerActionTargets(focusedTabId) {
  assert.ok(focusedTabId, "focused browser tab identity")
  return [{ kind: "desktop" }, { kind: "browser_tab", id: focusedTabId }]
}

export function assertRoomComputerActionRunning(
  action,
  { actionId, actorId, kind, focusedTabId },
) {
  assert.ok(action, `Room Action ${actionId} must exist`)
  assert.equal(action.action_id, actionId, "Room Action identity")
  assert.equal(action.actor_id, actorId, "Room Action actor")
  assert.equal(action.mode, "computer", "Room Action mode")
  assert.equal(action.kind, kind, "Room Action kind")
  assert.deepEqual(action.targets, computerActionTargets(focusedTabId), "Room Action targets")
  assert.equal(action.state, "running", "Room Action state")
  assert.equal(action.cancellation_requested, false, "Room Action cancellation flag")
  assert.ok(Number.isInteger(action.started_at_ms), "Room Action start timestamp")
  assert.equal(action.finished_at_ms, null, "running Room Action finish timestamp")
  assert.equal(action.outcome, null, "running Room Action outcome")
}

export function assertCancellationRequestedResponse(response, { actionId }) {
  assert.equal(response?.outcome?.state, "cancellation_requested", "cancellation outcome")
  const action = response?.environment?.actions?.find(
    (candidate) => candidate.action_id === actionId,
  )
  assert.ok(action, `cancellation response must retain Room Action ${actionId}`)
  assert.equal(action.state, "running", "cancelled execution remains reserved while stopping")
  assert.equal(action.cancellation_requested, true, "cancellation request must be durable")
  assert.equal(action.finished_at_ms, null, "cancellation response must not claim completion")
}

export function assertRoomComputerActionCancelled(
  action,
  { actionId, actorId, kind, focusedTabId },
) {
  assert.ok(action, `Room Action ${actionId} must exist`)
  assert.equal(action.action_id, actionId, "Room Action identity")
  assert.equal(action.actor_id, actorId, "Room Action actor")
  assert.equal(action.mode, "computer", "Room Action mode")
  assert.equal(action.kind, kind, "Room Action kind")
  assert.deepEqual(action.targets, computerActionTargets(focusedTabId), "Room Action targets")
  assert.equal(action.state, "cancelled", "Room Action state")
  assert.equal(action.cancellation_requested, false, "terminal Action cancellation flag")
  assert.ok(Number.isInteger(action.started_at_ms), "Room Action start timestamp")
  assert.ok(Number.isInteger(action.finished_at_ms), "Room Action finish timestamp")
  assert.ok(action.finished_at_ms >= action.started_at_ms, "Room Action timestamp order")
  assert.deepEqual(
    action.outcome,
    { status: "cancelled", reason: "requested" },
    "Room Action cancellation outcome",
  )
}

export function roomComputerCancellationLatencyMs(action, requestedAtMs) {
  assert.ok(Number.isInteger(requestedAtMs), "cancellation request timestamp")
  assert.ok(Number.isInteger(action?.finished_at_ms), "terminal Action finish timestamp")
  assert.ok(
    action.finished_at_ms >= requestedAtMs,
    "terminal Action finished before its cancellation request",
  )
  return action.finished_at_ms - requestedAtMs
}

export function roomComputerCancellationTimings(
  action,
  { initiatedAtMs, requestObservedAtMs },
) {
  assert.ok(Number.isInteger(initiatedAtMs), "cancellation initiation timestamp")
  assert.ok(Number.isInteger(requestObservedAtMs), "cancellation request observation timestamp")
  assert.ok(
    requestObservedAtMs >= initiatedAtMs,
    "cancellation request was observed before initiation",
  )
  assert.ok(Number.isInteger(action?.finished_at_ms), "terminal Action finish timestamp")
  assert.ok(
    action.finished_at_ms >= requestObservedAtMs,
    "terminal Action finished before its cancellation request was observed",
  )
  return {
    dispatchLatencyMs: requestObservedAtMs - initiatedAtMs,
    physicalStopLatencyMs: action.finished_at_ms - requestObservedAtMs,
    endToEndLatencyMs: action.finished_at_ms - initiatedAtMs,
  }
}

export function assertHumanTakeoverCancellationRequired(
  response,
  { actionId, humanActorId },
) {
  assert.deepEqual(
    response?.outcome,
    { state: "cancellation_required", action_ids: [actionId] },
    "takeover cancellation requirement",
  )
  const environment = response.environment
  const action = environment.actions.find((candidate) => candidate.action_id === actionId)
  assert.equal(action?.state, "running", "takeover must not claim early Action completion")
  assert.equal(action?.cancellation_requested, true, "takeover must request Action cancellation")
  assert.equal(
    environment.input_ownership.some((entry) => entry.target?.kind === "desktop"),
    false,
    "human ownership must wait for physical input reset",
  )
  assert.deepEqual(environment.pending_input_takeovers, [{
    target: { kind: "desktop" },
    human_actor_id: humanActorId,
    blocking_action_ids: [actionId],
  }], "pending human takeover")
}

export function assertHumanDesktopTakeoverCompleted(
  environment,
  { actionId, humanActorId },
) {
  const action = environment.actions.find((candidate) => candidate.action_id === actionId)
  assert.equal(action?.state, "cancelled", "blocking Action must be cancelled before takeover")
  assert.equal(action?.cancellation_requested, false, "terminal cancellation flag")
  assert.deepEqual(action?.outcome, { status: "cancelled", reason: "requested" })
  assert.ok(environment.input_ownership.some((entry) => (
    entry.target?.kind === "desktop" && entry.actor_id === humanActorId
  )), "human must own desktop after cancellation and reset")
  assert.deepEqual(environment.pending_input_takeovers, [], "pending takeover must settle")
}

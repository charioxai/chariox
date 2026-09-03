import assert from "node:assert/strict"
import test from "node:test"

import {
  assertCancellationRequestedResponse,
  assertHumanDesktopTakeoverCompleted,
  assertHumanTakeoverCancellationRequired,
  assertRoomComputerActionCancelled,
  assertRoomComputerActionRunning,
  roomComputerCancellationLatencyMs,
} from "./room-environment-computer-cancellation-drill.mjs"

const runningAction = {
  action_id: "action-17",
  sequence: 17,
  actor_id: "agent:agent-2",
  runtime_generation: 4,
  mode: "computer",
  kind: "keyboard_text",
  targets: [{ kind: "desktop" }, { kind: "browser_tab", id: "tab-1" }],
  state: "running",
  cancellation_requested: false,
  submitted_at_ms: 100,
  started_at_ms: 101,
  finished_at_ms: null,
  outcome: null,
}

test("running Computer Action is attributable and non-terminal", () => {
  assertRoomComputerActionRunning(runningAction, {
    actionId: "action-17",
    actorId: "agent:agent-2",
    kind: "keyboard_text",
    focusedTabId: "tab-1",
  })
})

test("cancellation response keeps a running Action reserved until execution stops", () => {
  assertCancellationRequestedResponse({
    outcome: { state: "cancellation_requested" },
    environment: {
      actions: [{ ...runningAction, cancellation_requested: true }],
    },
  }, { actionId: "action-17" })
})

test("terminal cancelled Action clears its request and records requested outcome", () => {
  assertRoomComputerActionCancelled({
    ...runningAction,
    state: "cancelled",
    cancellation_requested: false,
    finished_at_ms: 120,
    outcome: { status: "cancelled", reason: "requested" },
  }, {
    actionId: "action-17",
    actorId: "agent:agent-2",
    kind: "keyboard_text",
    focusedTabId: "tab-1",
  })
})

test("cancellation latency uses the authoritative terminal timestamp", () => {
  assert.equal(roomComputerCancellationLatencyMs({ finished_at_ms: 1_450 }, 1_000), 450)
  assert.throws(
    () => roomComputerCancellationLatencyMs({ finished_at_ms: null }, 1_000),
    /finish timestamp/,
  )
  assert.throws(
    () => roomComputerCancellationLatencyMs({ finished_at_ms: 999 }, 1_000),
    /before its cancellation request/,
  )
})

test("human takeover remains pending until the blocking Action is cancelled", () => {
  assertHumanTakeoverCancellationRequired({
    outcome: { state: "cancellation_required", action_ids: ["action-17"] },
    environment: {
      actions: [{ ...runningAction, cancellation_requested: true }],
      input_ownership: [],
      pending_input_takeovers: [{
        target: { kind: "desktop" },
        human_actor_id: "user:local",
        blocking_action_ids: ["action-17"],
      }],
    },
  }, { actionId: "action-17", humanActorId: "user:local" })

  assertHumanDesktopTakeoverCompleted({
    actions: [{
      ...runningAction,
      state: "cancelled",
      finished_at_ms: 120,
      outcome: { status: "cancelled", reason: "requested" },
    }],
    input_ownership: [{ target: { kind: "desktop" }, actor_id: "user:local" }],
    pending_input_takeovers: [],
  }, { actionId: "action-17", humanActorId: "user:local" })
})

test("cancellation validators reject premature ownership and false completion", () => {
  assert.throws(
    () => assertRoomComputerActionCancelled(runningAction, {
      actionId: "action-17",
      actorId: "agent:agent-2",
      kind: "keyboard_text",
      focusedTabId: "tab-1",
    }),
    /state/,
  )
  assert.throws(
    () => assertHumanDesktopTakeoverCompleted({
      actions: [runningAction],
      input_ownership: [],
      pending_input_takeovers: [],
    }, { actionId: "action-17", humanActorId: "user:local" }),
    /cancelled/,
  )
})

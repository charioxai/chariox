import assert from "node:assert/strict"
import test from "node:test"

import { captureDrillEFromClient, verifyDrillE } from "./live-browser-computer-drill-e.mjs"

function action({ id, sequence, actor, tab, state, submitted, started = null, finished = null,
  cancellationRequested = false, outcome = null, kind = "click" }) {
  return {
    action_id: id,
    sequence,
    idempotency_key: null,
    actor_id: actor,
    runtime_generation: 4,
    mode: "browser",
    kind,
    targets: [{ kind: "browser_tab", id: tab }],
    state,
    cancellation_requested: cancellationRequested,
    submitted_at_ms: submitted,
    started_at_ms: started,
    finished_at_ms: finished,
    outcome,
  }
}

function environment({ actions, pending = [], ownership = [] }) {
  return {
    session_id: "session-live",
    environment_id: "environment-live",
    runtime_generation: 4,
    lifecycle: "ready",
    viewport: {
      css_width: 1280,
      css_height: 720,
      device_scale_factor: 1,
      desktop_pixel_width: 1280,
      desktop_pixel_height: 720,
      revision: 1,
      last_actor_id: null,
    },
    health: [],
    pointers: [],
    focused_tab_id: "tab:one",
    event_cursor: 31,
    actors: [
      { actor_id: "agent:a", kind: "agent", display_label: "A", presence: "present", presentation_color: "blue" },
      { actor_id: "agent:b", kind: "agent", display_label: "B", presence: "present", presentation_color: "cyan" },
      { actor_id: "agent:c", kind: "agent", display_label: "C", presence: "present", presentation_color: "green" },
      { actor_id: "user:operator", kind: "human", display_label: "Operator", presence: "present", presentation_color: "amber" },
    ],
    tabs: [
      { tab_id: "tab:one", url: "https://one.invalid", title: "One", document_revision: 1, focused: true },
      { tab_id: "tab:two", url: "https://two.invalid", title: "Two", document_revision: 1, focused: false },
    ],
    actions,
    pending_input_takeovers: pending,
    input_ownership: ownership,
  }
}

test("public LocalDaemon snapshot and Action history cannot claim tab reads when only mutations are recorded", async () => {
  const firstMutation = action({
    id: "action:first", sequence: 1, actor: "agent:a", tab: "tab:one",
    state: "running", submitted: 100, started: 100,
  })
  const queuedMutation = action({
    id: "action:queued", sequence: 2, actor: "agent:b", tab: "tab:one",
    state: "queued", submitted: 120,
  })
  const secondTabWork = action({
    id: "action:other-tab", sequence: 3, actor: "agent:c", tab: "tab:two",
    state: "running", submitted: 150, started: 150,
  })
  const queueSnapshot = environment({ actions: [firstMutation, queuedMutation, secondTabWork] })

  const waitingForHuman = {
    ...firstMutation,
    state: "running",
    cancellation_requested: true,
  }
  const cancelledQueuedMutation = {
    ...queuedMutation,
    state: "cancelled",
    finished_at_ms: 180,
    outcome: { status: "cancelled", reason: "human_takeover" },
  }
  const takeoverSnapshot = environment({
    actions: [waitingForHuman, cancelledQueuedMutation, secondTabWork],
    pending: [{
      target: { kind: "browser_tab", id: "tab:one" },
      human_actor_id: "user:operator",
      blocking_action_ids: ["action:first"],
    }],
  })
  const finalEnvironment = environment({
    actions: [
      {
        ...firstMutation,
        state: "cancelled",
        cancellation_requested: true,
        finished_at_ms: 190,
        outcome: { status: "cancelled", reason: "human_takeover" },
      },
      cancelledQueuedMutation,
      {
        ...secondTabWork,
        state: "completed",
        finished_at_ms: 220,
        outcome: { status: "completed" },
      },
    ],
    ownership: [{
      target: { kind: "browser_tab", id: "tab:one" },
      actor_id: "user:operator",
    }],
  })
  const history = [
    ...finalEnvironment.actions,
  ]

  const stateResponses = [environment({ actions: [] }), queueSnapshot, takeoverSnapshot, finalEnvironment]
  const sentRequests = []
  let stateRead = 0
  let historyRead = 0
  let clock = 100
  const requests = {
    getRoomEnvironmentStateRequest: (roomId) => ({ GetRoomEnvironmentState: { session_id: roomId } }),
    listRoomEnvironmentActionHistoryRequest: (roomId, before, limit) => ({
      ListRoomEnvironmentActionHistory: {
        session_id: roomId,
        before_sequence: before,
        limit,
      },
    }),
  }
  const client = {
    async send(request) {
      sentRequests.push(request)
      if ("GetRoomEnvironmentState" in request) {
        return { RoomEnvironmentState: { environment: stateResponses[stateRead++] } }
      }
      if ("ListRoomEnvironmentActionHistory" in request) {
        const pageActions = historyRead++ === 0 ? [] : history
        return { RoomEnvironmentActionHistoryListed: { page: { actions: pageActions, next_before_sequence: null } } }
      }
      throw new Error("unexpected public LocalDaemon request")
    },
  }

  const captured = await captureDrillEFromClient({
    client,
    requests,
    sessionId: "session-live",
    observeMs: 1,
    pollMs: 1,
    now: () => clock,
    sleep: async (duration) => { clock += duration },
  })

  const report = captured.report
  assert.equal(report.status, "incomplete")
  assert.equal(report.checks.twoAgentTabReads.status, "incomplete")
  assert.match(report.checks.twoAgentTabReads.reason, /status\/find observation/i)
  assert.equal(report.checks.sameTabSerialization.status, "passed")
  assert.equal(report.checks.independentTabConcurrency.status, "passed")
  assert.equal(report.checks.humanTakeover.status, "passed")
  assert.deepEqual(sentRequests.map((request) => Object.keys(request)[0]), [
    "GetRoomEnvironmentState",
    "ListRoomEnvironmentActionHistory",
    "GetRoomEnvironmentState",
    "GetRoomEnvironmentState",
    "ListRoomEnvironmentActionHistory",
    "GetRoomEnvironmentState",
  ])
})

test("two same-tab agent observations and third-tab work require matching completed kernel history", () => {
  const first = action({
    id: "action:read-a", sequence: 1, actor: "agent:a", tab: "tab:one",
    kind: "browser_status", state: "running", submitted: 100, started: 101,
  })
  const second = action({
    id: "action:read-b", sequence: 2, actor: "agent:b", tab: "tab:one",
    kind: "browser_find", state: "running", submitted: 102, started: 103,
  })
  const third = action({
    id: "action:other", sequence: 3, actor: "agent:c", tab: "tab:two",
    state: "running", submitted: 104, started: 105,
  })
  const observed = environment({ actions: [first, second, third] })
  const completed = [first, second, third].map((entry) => ({
    ...entry, state: "completed", finished_at_ms: 120, outcome: { status: "completed" },
  }))
  const finalEnvironment = environment({ actions: completed })
  const input = {
    sessionId: "session-live",
    snapshots: [{ observed_at_ms: 110, environment: observed }],
    finalEnvironment,
    actions: completed,
    baselineActionSequence: 0,
  }
  const report = verifyDrillE(input)
  assert.equal(report.checks.twoAgentTabReads.status, "passed")
  assert.deepEqual(report.checks.twoAgentTabReads.readActionIds, ["action:read-a", "action:read-b"])
  assert.equal(report.checks.twoAgentTabReads.thirdAgentActionId, "action:other")
  assert.equal(report.status, "incomplete", "mutation and takeover gates still require evidence")

  const wrongKind = completed.map((entry) => entry.action_id === "action:read-b"
    ? { ...entry, kind: "click" } : entry)
  assert.equal(verifyDrillE({ ...input, actions: wrongKind }).checks.twoAgentTabReads.status, "incomplete")
  const wrongActor = completed.map((entry) => entry.action_id === "action:read-b"
    ? { ...entry, actor_id: "agent:a" } : entry)
  assert.equal(verifyDrillE({ ...input, actions: wrongActor }).checks.twoAgentTabReads.status, "incomplete")
  const missingThird = environment({ actions: [first, second] })
  assert.equal(verifyDrillE({
    ...input, snapshots: [{ observed_at_ms: 110, environment: missingThird }],
  }).checks.twoAgentTabReads.status, "incomplete")
})

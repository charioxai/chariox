import assert from "node:assert/strict"
import test from "node:test"

import type {
  RoomEnvironmentEvent,
  RoomEnvironmentSnapshot,
} from "@chariox/kernel-client/kernel-types"

import {
  createRoomEnvironmentActivityController,
} from "./room-environment-activity-controller.js"

test("Room activity loads once and empty replay does not add transcript noise", async () => {
  const environment = roomEnvironment()
  const harness = activityHarness([
    { RoomEnvironmentState: { environment } },
    { RoomEnvironmentEvents: { replay: { Events: { events: [], next_cursor: 4 } } } },
  ])

  assert.equal(await harness.controller.synchronize(), true)
  assert.equal(await harness.controller.synchronize(), false)

  assert.deepEqual(harness.requests, [
    { GetRoomEnvironmentState: { session_id: "session-1" } },
    { GetRoomEnvironmentEvents: { session_id: "session-1", cursor: 4 } },
  ])
  assert.deepEqual(harness.notices, [
    "Room screen: ready · tab Docs — https://example.test/docs · actors Mara, Miguel · input Mara",
  ])
  assert.deepEqual(harness.activity, ["room_environment_state"])
})

test("Room activity projects actor, tab, input, and action outcome events without pointer spam", async () => {
  const initial = roomEnvironment()
  const changed = roomEnvironment({
    eventCursor: 9,
    desktopOwnerActorId: "user:miguel",
    actions: [{
      action_id: "action-1",
      sequence: 1,
      idempotency_key: null,
      actor_id: "agent:agent-1",
      runtime_generation: 2,
      mode: "browser",
      kind: "navigate",
      targets: [{ kind: "browser_tab", id: "tab-1" }],
      state: "completed",
      cancellation_requested: false,
      submitted_at_ms: 10,
      started_at_ms: 11,
      finished_at_ms: 12,
      outcome: { status: "completed" },
    }],
  })
  const events: RoomEnvironmentEvent[] = [
    roomEvent(5, "ActorsChanged"),
    roomEvent(6, "PointersChanged"),
    roomEvent(7, "TabsChanged"),
    roomEvent(8, "InputOwnershipChanged"),
    roomEvent(9, {
      ActionChanged: {
        action_id: "action-1",
        state: "completed",
        cancellation_requested: false,
        submitted_at_ms: 10,
        started_at_ms: 11,
        finished_at_ms: 12,
        outcome: { status: "completed" },
      },
    }),
  ]
  const harness = activityHarness([
    { RoomEnvironmentState: { environment: initial } },
    { RoomEnvironmentEvents: { replay: { Events: { events, next_cursor: 9 } } } },
    { RoomEnvironmentState: { environment: changed } },
  ])

  await harness.controller.synchronize()
  harness.notices.length = 0
  harness.activity.length = 0

  assert.equal(await harness.controller.synchronize(), true)
  assert.deepEqual(harness.notices, [
    "Room actors: Mara (present), Miguel (present)",
    "Room tab: Docs — https://example.test/docs",
    "Room input: Miguel controls desktop",
    "Room action #1: Mara · browser navigate · completed",
  ])
  assert.deepEqual(harness.activity, ["room_environment_events"])
  assert.equal(harness.requests.length, 3, "one state refresh should serve the complete event batch")
})

test("Room activity keeps consecutive same-kind Actions distinct by sequence", async () => {
  const initial = roomEnvironment()
  const actions: RoomEnvironmentSnapshot["actions"] = [1, 2].map((sequence) => ({
    action_id: `action-${sequence}`,
    sequence,
    idempotency_key: null,
    actor_id: "agent:agent-1",
    runtime_generation: 2,
    mode: "computer",
    kind: "pointer_click",
    arguments: {
      kind: "pointer_click",
      x: 640,
      y: 400,
      button: "left",
      click_count: 1,
      viewport_revision: 3,
    },
    targets: [{ kind: "desktop" }],
    state: "completed",
    cancellation_requested: false,
    submitted_at_ms: sequence * 10,
    started_at_ms: sequence * 10 + 1,
    finished_at_ms: sequence * 10 + 2,
    outcome: { status: "completed" },
  }))
  const events: RoomEnvironmentEvent[] = actions.map((action, index) => roomEvent(5 + index, {
    ActionChanged: {
      action_id: action.action_id,
      state: "completed",
      cancellation_requested: false,
      submitted_at_ms: action.submitted_at_ms,
      started_at_ms: action.started_at_ms,
      finished_at_ms: action.finished_at_ms,
      outcome: { status: "completed" },
    },
  }))
  const harness = activityHarness([
    { RoomEnvironmentState: { environment: initial } },
    { RoomEnvironmentEvents: { replay: { Events: { events, next_cursor: 6 } } } },
    { RoomEnvironmentState: { environment: roomEnvironment({ eventCursor: 6, actions }) } },
  ])

  await harness.controller.synchronize()
  harness.notices.length = 0
  await harness.controller.synchronize()

  assert.deepEqual(harness.notices, [
    "Room action #1: Mara · computer pointer_click · completed",
    "Room action #2: Mara · computer pointer_click · completed",
  ])
})

test("Room activity applies a replay-gap snapshot directly", async () => {
  const initial = roomEnvironment()
  const snapshot = roomEnvironment({ eventCursor: 21, lifecycle: "degraded" })
  const harness = activityHarness([
    { RoomEnvironmentState: { environment: initial } },
    { RoomEnvironmentEvents: { replay: { SnapshotRequired: { snapshot } } } },
  ])

  await harness.controller.synchronize()
  harness.notices.length = 0

  assert.equal(await harness.controller.synchronize(), true)
  assert.deepEqual(harness.notices, [
    "Room activity resynchronized: degraded · tab Docs — https://example.test/docs · actors Mara, Miguel · input Mara",
  ])
  assert.equal(harness.requests.length, 2, "snapshot replay must not trigger a redundant state request")
})

test("Room activity reports released desktop input without claiming available is an actor", async () => {
  const harness = activityHarness([
    { RoomEnvironmentState: { environment: roomEnvironment() } },
    {
      RoomEnvironmentEvents: {
        replay: {
          Events: {
            events: [roomEvent(5, "InputOwnershipChanged")],
            next_cursor: 5,
          },
        },
      },
    },
    {
      RoomEnvironmentState: {
        environment: roomEnvironment({ eventCursor: 5, desktopOwnerActorId: null }),
      },
    },
  ])

  await harness.controller.synchronize()
  harness.notices.length = 0
  await harness.controller.synchronize()

  assert.deepEqual(harness.notices, ["Room input: available"])
})

test("Room activity treats a missing environment as idle and probes again on a bounded interval", async () => {
  const unavailable = Object.assign(new Error("Room has no Environment"), {
    code: "environment_not_found",
  })
  const harness = activityHarness([
    unavailable,
    unavailable,
  ])

  assert.equal(await harness.controller.synchronize(), false)
  harness.nowMs = 4_999
  assert.equal(await harness.controller.synchronize(), false)
  assert.equal(harness.requests.length, 1)
  harness.nowMs = 5_000
  assert.equal(await harness.controller.synchronize(), false)
  assert.equal(harness.requests.length, 2)
  assert.deepEqual(harness.notices, [])
})

test("Room activity rejects replay cursor rollback", async () => {
  const harness = activityHarness([
    { RoomEnvironmentState: { environment: roomEnvironment() } },
    { RoomEnvironmentEvents: { replay: { Events: { events: [], next_cursor: 3 } } } },
  ])

  await harness.controller.synchronize()
  await assert.rejects(
    harness.controller.synchronize(),
    /cursor moved backwards from 4 to 3/,
  )
})

test("late Room state cannot overwrite a newly attached session", async () => {
  let resolveFirst!: (value: unknown) => void
  const firstResponse = new Promise((resolve) => {
    resolveFirst = resolve
  })
  let sessionId = "session-1"
  const notices: string[] = []
  const controller = createRoomEnvironmentActivityController({
    isAttached: () => true,
    sessionId: () => sessionId,
    nowMs: () => 0,
    send: async <T>(request: unknown): Promise<T> => {
      if ((request as { GetRoomEnvironmentState?: { session_id?: string } })
        .GetRoomEnvironmentState?.session_id === "session-1") {
        return await firstResponse as T
      }
      return {
        RoomEnvironmentState: {
          environment: roomEnvironment({ sessionId: "session-2", lifecycle: "starting" }),
        },
      } as T
    },
    appendNotice: (message) => notices.push(message),
    recordDaemonActivity: () => {},
  })

  const first = controller.synchronize()
  sessionId = "session-2"
  assert.equal(await controller.synchronize(), true)
  resolveFirst({ RoomEnvironmentState: { environment: roomEnvironment() } })
  assert.equal(await first, false)

  assert.deepEqual(notices, [
    "Room screen: starting · tab Docs — https://example.test/docs · actors Mara, Miguel · input Mara",
  ])
})

function activityHarness(responses: unknown[]) {
  const harness = {
    attached: true,
    sessionId: "session-1",
    nowMs: 0,
    requests: [] as unknown[],
    notices: [] as string[],
    activity: [] as string[],
    controller: null as ReturnType<typeof createRoomEnvironmentActivityController> | null,
  }
  harness.controller = createRoomEnvironmentActivityController({
    isAttached: () => harness.attached,
    sessionId: () => harness.sessionId,
    nowMs: () => harness.nowMs,
    send: async <T>(request: unknown): Promise<T> => {
      harness.requests.push(request)
      const response = responses.shift()
      if (response instanceof Error) throw response
      if (response === undefined) throw new Error("unexpected Room activity request")
      return response as T
    },
    appendNotice: (message) => harness.notices.push(message),
    recordDaemonActivity: (kind) => harness.activity.push(kind),
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createRoomEnvironmentActivityController>
  }
}

function roomEnvironment(options: {
  sessionId?: string
  eventCursor?: number
  lifecycle?: RoomEnvironmentSnapshot["lifecycle"]
  desktopOwnerActorId?: string | null
  actions?: RoomEnvironmentSnapshot["actions"]
} = {}): RoomEnvironmentSnapshot {
  return {
    session_id: options.sessionId ?? "session-1",
    environment_id: "environment-1",
    runtime_generation: 2,
    lifecycle: options.lifecycle ?? "ready",
    health: [
      { component: "browser_controller", state: "ready", diagnostic_code: null },
      { component: "browser", state: "ready", diagnostic_code: null },
      { component: "desktop", state: "ready", diagnostic_code: null },
      { component: "streamer", state: "ready", diagnostic_code: null },
    ],
    viewport: {
      css_width: 1280,
      css_height: 720,
      device_scale_factor: 1,
      desktop_pixel_width: 1280,
      desktop_pixel_height: 720,
      revision: 3,
      last_actor_id: "agent:agent-1",
    },
    actors: [
      {
        actor_id: "agent:agent-1",
        kind: "agent",
        display_label: "Mara",
        presence: "present",
        presentation_color: "blue",
      },
      {
        actor_id: "user:miguel",
        kind: "human",
        display_label: "Miguel",
        presence: "present",
        presentation_color: "green",
      },
    ],
    pointers: [],
    tabs: [{
      tab_id: "tab-1",
      url: "https://example.test/docs",
      title: "Docs",
      document_revision: 4,
      focused: true,
    }],
    focused_tab_id: "tab-1",
    actions: options.actions ?? [],
    input_ownership: options.desktopOwnerActorId === null ? [] : [{
      target: { kind: "desktop" },
      actor_id: options.desktopOwnerActorId ?? "agent:agent-1",
    }],
    pending_input_takeovers: [],
    event_cursor: options.eventCursor ?? 4,
  }
}

function roomEvent(
  eventId: number,
  kind: RoomEnvironmentEvent["kind"],
): RoomEnvironmentEvent {
  return {
    event_id: eventId,
    environment_id: "environment-1",
    runtime_generation: 2,
    kind,
  }
}

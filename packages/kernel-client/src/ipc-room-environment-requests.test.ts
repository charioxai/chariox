import assert from "node:assert/strict"
import test from "node:test"

import {
  bindRoomEnvironmentSliceRequest,
  getRoomEnvironmentSliceRequest,
  cancelRoomEnvironmentActionRequest,
  getRoomEnvironmentEventsRequest,
  getRoomEnvironmentStateRequest,
  listRoomEnvironmentActionHistoryRequest,
  requestRoomEnvironmentInputTakeoverRequest,
  releaseRoomEnvironmentInputRequest,
  startRoomEnvironmentRequest,
  stopRoomEnvironmentRequest,
  updateRoomEnvironmentViewportRequest,
  retryRoomEnvironmentRequest,
} from "./ipc-room-environment-requests.js"
import type {
  RoomEnvironmentActionCancellationOutcome,
  RoomEnvironmentActionHistoryResponse,
  RoomEnvironmentEventsResponse,
  RoomEnvironmentStateResponse,
  RoomEnvironmentUpdatedResponse,
} from "./kernel-types-environment.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("Room Environment placement uses shared requests", () => {
  assert.deepEqual(bindRoomEnvironmentSliceRequest("session-1", "desktop"), {
    BindRoomEnvironmentSlice: { session_id: "session-1", slice_ref: "desktop" },
  })
  assert.deepEqual(getRoomEnvironmentSliceRequest("session-1"), {
    GetRoomEnvironmentSlice: { session_id: "session-1" },
  })
})

test("Room Environment state request matches protocol 290", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 293)
  assert.deepEqual(getRoomEnvironmentStateRequest("session-1"), {
    GetRoomEnvironmentState: {
      session_id: "session-1",
    },
  })

  const response: RoomEnvironmentStateResponse = {
    RoomEnvironmentState: {
      environment: {
        session_id: "session-1",
        environment_id: "environment-1",
        runtime_generation: 1,
        lifecycle: "ready",
        health: [
          {
            component: "browser_controller",
            state: "ready",
            diagnostic_code: null,
          },
        ],
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 1,
          desktop_pixel_width: 1280,
          desktop_pixel_height: 800,
          revision: 1,
          last_actor_id: "human-1",
        },
        actors: [
          {
            actor_id: "human-1",
            kind: "human",
            display_label: "Miguel",
            presence: "present",
          },
        ],
        tabs: [
          {
            tab_id: "tab-1",
            url: "https://example.test/",
            title: "Example",
            document_revision: 3,
            focused: true,
          },
        ],
        focused_tab_id: "tab-1",
        actions: [
          {
            action_id: "action-1",
            sequence: 1,
            idempotency_key: "idempotency-1",
            actor_id: "human-1",
            runtime_generation: 1,
            mode: "browser",
            kind: "click",
            targets: [
              { kind: "desktop" },
              { kind: "browser_tab", id: "tab-1" },
            ],
            state: "completed",
            cancellation_requested: false,
            submitted_at_ms: 40,
            started_at_ms: 40,
            finished_at_ms: 44,
            outcome: { status: "completed" },
          },
          {
            action_id: "action-2",
            sequence: 2,
            idempotency_key: null,
            actor_id: "human-1",
            runtime_generation: 1,
            mode: "browser",
            kind: "second-click",
            targets: [{ kind: "browser_tab", id: "tab-1" }],
            state: "queued",
            cancellation_requested: false,
            submitted_at_ms: 45,
            started_at_ms: null,
            finished_at_ms: null,
            outcome: null,
          },
        ],
        input_ownership: [
          {
            target: { kind: "desktop" },
            actor_id: "human-1",
          },
        ],
        pending_input_takeovers: [],
        event_cursor: 7,
      },
    },
  }
  assert.equal(response.RoomEnvironmentState.environment.tabs[0]?.tab_id, "tab-1")
})

test("Room Environment event replay request matches protocol 290", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 293)
  assert.deepEqual(getRoomEnvironmentEventsRequest("session-1", 41), {
    GetRoomEnvironmentEvents: {
      session_id: "session-1",
      cursor: 41,
    },
  })

  const response: RoomEnvironmentEventsResponse = {
    RoomEnvironmentEvents: {
      replay: {
        Events: {
          events: [
            {
              event_id: 42,
              environment_id: "environment-1",
              runtime_generation: 3,
              kind: { ViewportChanged: { revision: 7 } },
            },
          ],
          next_cursor: 42,
        },
      },
    },
  }
  assert.deepEqual(response.RoomEnvironmentEvents.replay, {
    Events: {
      events: [
        {
          event_id: 42,
          environment_id: "environment-1",
          runtime_generation: 3,
          kind: { ViewportChanged: { revision: 7 } },
        },
      ],
      next_cursor: 42,
    },
  })
})

test("Room Environment Action history request matches protocol 290", () => {
  assert.deepEqual(listRoomEnvironmentActionHistoryRequest("session-1", 42, 25), {
    ListRoomEnvironmentActionHistory: {
      session_id: "session-1",
      before_sequence: 42,
      limit: 25,
    },
  })

  const response: RoomEnvironmentActionHistoryResponse = {
    RoomEnvironmentActionHistoryListed: {
      page: {
        actions: [],
        next_before_sequence: null,
      },
    },
  }
  assert.deepEqual(response.RoomEnvironmentActionHistoryListed.page.actions, [])
})

test("Room Environment start request keeps viewport ownership at the kernel seam", () => {
  assert.deepEqual(
    startRoomEnvironmentRequest("session-1", {
      css_width: 1280,
      css_height: 800,
      device_scale_factor: 2,
      desktop_pixel_width: 2560,
      desktop_pixel_height: 1600,
    }),
    {
      StartRoomEnvironment: {
        session_id: "session-1",
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 2,
          desktop_pixel_width: 2560,
          desktop_pixel_height: 1600,
        },
      },
    },
  )

  const response: RoomEnvironmentUpdatedResponse = {
    RoomEnvironmentUpdated: {
      environment: {
        session_id: "session-1",
        environment_id: "environment-session-1",
        runtime_generation: 1,
        lifecycle: "starting",
        health: [],
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 2,
          desktop_pixel_width: 2560,
          desktop_pixel_height: 1600,
          revision: 1,
          last_actor_id: null,
        },
        actors: [],
        tabs: [],
        focused_tab_id: null,
        actions: [],
        input_ownership: [],
        pending_input_takeovers: [],
        event_cursor: 1,
      },
    },
  }
  assert.equal(response.RoomEnvironmentUpdated.environment.lifecycle, "starting")
})

test("Room Environment stop request uses the shared lifecycle seam", () => {
  assert.deepEqual(stopRoomEnvironmentRequest("session-1"), {
    StopRoomEnvironment: {
      session_id: "session-1",
    },
  })
})

test("Room Environment retry request uses the shared lifecycle seam", () => {
  assert.deepEqual(retryRoomEnvironmentRequest("session-1"), {
    RetryRoomEnvironment: {
      session_id: "session-1",
    },
  })
})

test("Room Environment viewport update carries only dimensions and observed revision", () => {
  assert.deepEqual(
    updateRoomEnvironmentViewportRequest(
      "session-1",
      4,
      {
        css_width: 1440,
        css_height: 900,
        device_scale_factor: 2,
        desktop_pixel_width: 2880,
        desktop_pixel_height: 1800,
      },
    ),
    {
      UpdateRoomEnvironmentViewport: {
        session_id: "session-1",
        expected_revision: 4,
        viewport: {
          css_width: 1440,
          css_height: 900,
          device_scale_factor: 2,
          desktop_pixel_width: 2880,
          desktop_pixel_height: 1800,
        },
      },
    },
  )
})

test("Room Environment takeover request cannot forge Actor identity", () => {
  assert.deepEqual(
    requestRoomEnvironmentInputTakeoverRequest("session-1", { kind: "desktop" }),
    {
      RequestRoomEnvironmentInputTakeover: {
        session_id: "session-1",
        target: { kind: "desktop" },
      },
    },
  )
})

test("Room Environment input release request cannot forge Actor identity", () => {
  assert.deepEqual(
    releaseRoomEnvironmentInputRequest("session-1", { kind: "desktop" }),
    {
      ReleaseRoomEnvironmentInput: {
        session_id: "session-1",
        target: { kind: "desktop" },
      },
    },
  )
})

test("Room Environment Action cancellation request cannot forge Actor identity", () => {
  assert.deepEqual(cancelRoomEnvironmentActionRequest("session-1", "action-7"), {
    CancelRoomEnvironmentAction: {
      session_id: "session-1",
      action_id: "action-7",
    },
  })

  const outcome: RoomEnvironmentActionCancellationOutcome = {
    state: "cancellation_requested",
  }
  assert.equal(outcome.state, "cancellation_requested")
})

import assert from "node:assert/strict"
import test from "node:test"

import { getRoomEnvironmentStateRequest } from "./ipc-room-environment-requests.js"
import type { RoomEnvironmentStateResponse } from "./kernel-types-environment.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("Room Environment state request matches protocol 269", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 269)
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
          },
        ],
        input_ownership: [
          {
            target: { kind: "desktop" },
            actor_id: "human-1",
          },
        ],
        event_cursor: 7,
      },
    },
  }
  assert.equal(response.RoomEnvironmentState.environment.tabs[0]?.tab_id, "tab-1")
})

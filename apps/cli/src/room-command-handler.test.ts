import assert from "node:assert/strict"
import test from "node:test"

import type { RoomEnvironmentSnapshot } from "@chariox/kernel-client/kernel-types"

import { parseSlashCommand } from "./commands.js"
import { handleRoomSlashCommand } from "./room-command-handler.js"

test("/room status reads and renders the attached Room environment", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room status")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentState: { environment: roomEnvironment() } } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{ GetRoomEnvironmentState: { session_id: "session-1" } }])
  assert.deepEqual(notices, [[
    "Room environment environment-1",
    "lifecycle=ready generation=2 cursor=4",
    "health=browser_controller:ready, browser:ready, desktop:ready, streamer:ready",
    "viewport=1280x720 css=1280x720 scale=1 revision=3",
    "tab=tab-1 Docs — https://example.test/docs",
    "actors=Mara (agent,present), Miguel (human,present)",
    "input=desktop:Mara",
    "last_action=none",
  ].join("\n")])
})

function roomEnvironment(): RoomEnvironmentSnapshot {
  return {
    session_id: "session-1",
    environment_id: "environment-1",
    runtime_generation: 2,
    lifecycle: "ready",
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
    actions: [],
    input_ownership: [{ target: { kind: "desktop" }, actor_id: "agent:agent-1" }],
    pending_input_takeovers: [],
    event_cursor: 4,
  }
}

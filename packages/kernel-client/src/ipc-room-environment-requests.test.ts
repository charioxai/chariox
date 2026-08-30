import assert from "node:assert/strict"
import test from "node:test"

import { getRoomEnvironmentStateRequest } from "./ipc-room-environment-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("Room Environment state request matches protocol 269", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 269)
  assert.deepEqual(getRoomEnvironmentStateRequest("session-1"), {
    GetRoomEnvironmentState: {
      session_id: "session-1",
    },
  })
})

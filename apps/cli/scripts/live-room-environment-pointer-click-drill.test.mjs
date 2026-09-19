import assert from "node:assert/strict"
import test from "node:test"

import {
  parseRoomRealProviderEffort,
  withRoomRealProviderEffort,
} from "./live-room-environment-pointer-click-drill.mjs"

function requestBuilders(calls) {
  return {
    spawnAgentRequest(...args) {
      calls.push(args)
      return {
        SpawnAgent: {
          session_id: args[0],
          provider: args[1],
          alias: args[2],
          model: args[3],
          effort: args[5],
        },
      }
    },
    unrelatedRequest: () => ({ Unrelated: true }),
  }
}

test("real-provider effort parses max and reaches the SpawnAgent request", () => {
  const calls = []
  const effort = parseRoomRealProviderEffort({ CHARIOX_ROOM_DRILL_EFFORT: " max " })
  const requests = withRoomRealProviderEffort(requestBuilders(calls), effort)

  const request = requests.spawnAgentRequest(
    "session-1", "codex", "real-codex", "gpt-5.6-luna", "/workspace",
    "low", "build", "yolo", undefined, undefined, "slice-1", "default",
  )

  assert.equal(request.SpawnAgent.effort, "max")
  assert.equal(calls.length, 1)
  assert.equal(calls[0][5], "max")
  assert.equal(requests.unrelatedRequest().Unrelated, true)
})

test("real-provider effort defaults to low for compatibility", () => {
  assert.equal(parseRoomRealProviderEffort({}), "low")
  assert.equal(parseRoomRealProviderEffort({ CHARIOX_ROOM_DRILL_EFFORT: "low" }), "low")

  const calls = []
  const requests = withRoomRealProviderEffort(requestBuilders(calls), parseRoomRealProviderEffort({}))
  requests.spawnAgentRequest("session-1", "codex", "real-codex", "gpt-5.6-luna", "/workspace", "low")
  assert.equal(calls[0][5], "low")
})

test("invalid real-provider effort fails closed before a request can be built", () => {
  assert.throws(
    () => parseRoomRealProviderEffort({ CHARIOX_ROOM_DRILL_EFFORT: "maximum" }),
    /CHARIOX_ROOM_DRILL_EFFORT must be one of low, medium, high, xhigh, max/,
  )
  assert.throws(
    () => withRoomRealProviderEffort(requestBuilders([]), "maximum"),
    /invalid Room real-provider effort/,
  )
})

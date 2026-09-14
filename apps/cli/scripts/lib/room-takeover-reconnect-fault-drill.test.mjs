import assert from "node:assert/strict"
import test from "node:test"

import {
  ROOM_TAKEOVER_RECONNECT_CASE_IDS,
  ROOM_TAKEOVER_RECONNECT_TEST_NAME,
  buildRoomTakeoverReconnectCargoArgs,
  parseRoomTakeoverReconnectProbe,
} from "./room-takeover-reconnect-fault-drill.mjs"

const probe = {
  schema: "chariox.room_takeover_reconnect_probe.v1",
  responseLostAfterCommit: true,
  replayedResponseMatched: true,
  humanOwnershipRetained: true,
  agentMutationBlocked: true,
  takeoverAppliedExactlyOnce: true,
  explicitReleaseRequired: true,
  agentMutationAdmittedAfterRelease: true,
  cleanupComplete: true,
  takeoverEventCount: 1,
}

test("Room takeover reconnect drill runs only the exact kernel authority probe", () => {
  assert.deepEqual(buildRoomTakeoverReconnectCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    ROOM_TAKEOVER_RECONNECT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(ROOM_TAKEOVER_RECONNECT_CASE_IDS, [
    "fault.takeover-response-loss",
    "reconnect.command-replay",
    "authority.human-retained",
    "authority.agent-blocked",
    "authority.explicit-release",
    "effect.takeover-exactly-once",
    "cleanup.resources",
  ])
})

test("Room takeover reconnect drill requires retained authority and one takeover event", () => {
  assert.deepEqual(
    parseRoomTakeoverReconnectProbe(`noise\nCHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  assert.throws(
    () => parseRoomTakeoverReconnectProbe(`CHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:${JSON.stringify({ ...probe, humanOwnershipRetained: false })}`),
    /humanOwnershipRetained must be true/,
  )
  assert.throws(
    () => parseRoomTakeoverReconnectProbe(`CHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:${JSON.stringify({ ...probe, takeoverEventCount: 2 })}`),
    /takeoverEventCount must be 1/,
  )
  assert.throws(
    () => parseRoomTakeoverReconnectProbe("test result: ok"),
    /missing chariox\.room_takeover_reconnect_probe\.v1/,
  )
})

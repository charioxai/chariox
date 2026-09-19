import assert from "node:assert/strict"
import test from "node:test"

import {
  parseRoomRealProviderEffort,
  spawnAndVerifyRealProviderAgent,
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

function stateFixture(effectiveEffort, requestEffort = effectiveEffort) {
  const calls = []
  const requests = withRoomRealProviderEffort({
    spawnAgentRequest(...args) {
      calls.push({ kind: "spawn", args })
      return { SpawnAgent: { effort: args[5] } }
    },
    getSessionStateRequest(sessionId) {
      calls.push({ kind: "state-request", sessionId })
      return { GetSessionState: { session_id: sessionId } }
    },
  }, requestEffort)
  const client = {
    send: async (request) => {
      if ("SpawnAgent" in request) {
        return { AgentSpawned: { agent: { id: "agent-1" } } }
      }
      return {
        SessionState: {
          session: {
            agents: [{
              id: "agent-1",
              provider: "codex",
              model: "gpt-5.6-luna",
              account_profile: "default",
              effort: effectiveEffort,
              is_processing: false,
            }],
          },
        },
      }
    },
  }
  return { client, requests, calls }
}

const providerOptions = {
  provider: "codex",
  model: "gpt-5.6-luna",
  accountProfile: "default",
}

test("real-provider effort parses max, reaches SpawnAgent, and matches public state", async () => {
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

  const fixture = stateFixture("max")
  const agent = await spawnAndVerifyRealProviderAgent({
    ...fixture,
    sessionId: "session-1",
    sliceId: "slice-1",
    workspace: "/workspace",
    options: providerOptions,
    effort,
  })
  assert.equal(agent.effort, "max")
  assert.equal(fixture.calls[0].args[5], "max")
})

test("real-provider effort fails closed when public state reports a mismatch", async () => {
  const fixture = stateFixture("low", "max")
  await assert.rejects(
    spawnAndVerifyRealProviderAgent({
      ...fixture,
      sessionId: "session-1",
      sliceId: "slice-1",
      workspace: "/workspace",
      options: providerOptions,
      effort: "max",
    }),
    /SessionState effective effort low did not match requested max/,
  )
  assert.equal(fixture.calls[0].args[5], "max")
})

test("real-provider effort defaults to low for compatibility", async () => {
  assert.equal(parseRoomRealProviderEffort({}), "low")
  assert.equal(parseRoomRealProviderEffort({ CHARIOX_ROOM_DRILL_EFFORT: "low" }), "low")

  const fixture = stateFixture("low")
  const agent = await spawnAndVerifyRealProviderAgent({
    ...fixture,
    sessionId: "session-1",
    sliceId: "slice-1",
    workspace: "/workspace",
    options: providerOptions,
    effort: parseRoomRealProviderEffort({}),
  })
  assert.equal(agent.effort, "low")
  assert.equal(fixture.calls[0].args[5], "low")
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

import assert from "node:assert/strict"
import test from "node:test"

import {
  assertRoomSharedBrowserReconnectPreserved,
  roomSharedBrowserReconnectSnapshot,
  roomSharedBrowserStatusNotice,
} from "./room-shared-browser-reconnect.mjs"

test("shared Browser reconnect identity retains Room, provider thread, profile, and both action histories", () => {
  const before = makeSnapshot()
  const after = structuredClone(before)
  assert.equal(assertRoomSharedBrowserReconnectPreserved(before, after), true)
})

for (const [label, mutate, message] of [
  ["Room", snapshot => { snapshot.sessionId = "other-room" }, /Room identity changed/],
  ["Browser tab", snapshot => { snapshot.environment.tabs[0].documentRevision++ }, /tab state changed/],
  ["provider profile", snapshot => { snapshot.agent.accountProfile = "other-profile" }, /account profile changed/],
  ["provider thread", snapshot => { snapshot.providerThread.providerSessionId = "other-thread" }, /thread\/history identity changed/],
  ["action history", snapshot => { snapshot.actions[1].actorId = "agent:wrong" }, /action history changed/],
]) {
  test(`shared Browser reconnect rejects changed ${label} identity`, () => {
    const before = makeSnapshot()
    const after = structuredClone(before)
    mutate(after)
    assert.throws(() => assertRoomSharedBrowserReconnectPreserved(before, after), message)
  })
}

test("shared Browser reconnect identity requires one provider thread and ordered Browser then human Computer actions", () => {
  const input = makeInput()
  assert.equal(roomSharedBrowserReconnectSnapshot(input).providerThread.providerSessionId, "thread-1")

  const noThread = makeInput()
  noThread.turns[0].external_provider_session_id = null
  assert.throws(() => roomSharedBrowserReconnectSnapshot(noThread), /completed provider turn with a thread identity/)

  const reversed = makeInput()
  reversed.actionHistory[0].sequence = 3
  assert.throws(() => roomSharedBrowserReconnectSnapshot(reversed), /must follow the structured Browser action/)
})

test("relay reconnect needs a new remote TUI status notice for the same Environment and tab", () => {
  const beforeIds = [1, 2]
  const identity = makeSnapshot()
  const entries = [
    { id: 2, text: "Room environment environment-1\nlifecycle=ready tab=tab-1 Room pointer drill" },
    { id: 3, text: "Room environment environment-other\nlifecycle=ready tab=tab-1 Room pointer drill" },
    { id: 4, text: "Room environment environment-1\nlifecycle=ready tab=tab-other Room pointer drill" },
    { id: 5, text: "Room environment environment-1\nlifecycle=ready tab=tab-1 Room pointer drill" },
  ]
  assert.deepEqual(roomSharedBrowserStatusNotice(entries, beforeIds, identity), entries[3])
})

function makeSnapshot() {
  return roomSharedBrowserReconnectSnapshot(makeInput())
}

function makeInput() {
  const environment = {
    session_id: "room-1",
    environment_id: "environment-1",
    runtime_generation: 8,
    lifecycle: "ready",
    focused_tab_id: "tab-1",
    tabs: [{ tab_id: "tab-1", url: "https://fixture.example/click", title: "Room pointer drill",
      document_revision: 4, focused: true }],
  }
  const session = {
    id: "room-1",
    agents: [{ id: "agent-1", session_id: "room-1", provider: "codex", model: "gpt-6-luna",
      account_profile: "profile-1" }],
  }
  const providerEvidence = {
    agentId: "agent-1", provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1",
    mode: "browser", actorId: "agent:agent-1", actionId: "browser-action-1",
  }
  const turns = [{ turn_id: "turn-1", lifecycle: "completed", external_provider: "codex",
    external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-1" }]
  const actionHistory = [
    { action_id: "browser-action-1", sequence: 1, actor_id: "agent:agent-1", runtime_generation: 8,
      mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }] },
    { action_id: "computer-action-1", sequence: 2, actor_id: "user:local", runtime_generation: 8,
      mode: "computer", kind: "pointer_click", state: "completed", targets: [{ kind: "desktop" }] },
  ]
  return {
    sessionId: "room-1", environment, session, providerAgentId: "agent-1", providerEvidence,
    computerActionId: "computer-action-1", computerActorId: "user:local", provider: "codex",
    model: "gpt-6-luna", accountProfile: "profile-1", turns, actionHistory,
  }
}

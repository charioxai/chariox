import assert from "node:assert/strict"
import test from "node:test"

import { roomSharedBrowserReconnectSnapshot } from "./room-shared-browser-reconnect.mjs"
import {
  assertRoomSharedBrowserClickAdvancedOnce,
  assertRoomSharedBrowserPersistence,
  roomSharedBrowserClickCount,
} from "./room-shared-browser-persistence.mjs"

test("save and restore retains Room, Browser tab, provider thread/profile, and actions before continuing", () => {
  const input = makeInput()
  assert.equal(assertRoomSharedBrowserPersistence(input), true)
})

for (const [label, mutate, message] of [
  ["Room", input => { input.environment.environment_id = "other-environment" }, /environment identity changed/],
  ["provider profile", input => { input.session.agents[0].account_profile = "other-profile" }, /agent or profile changed/],
  ["provider thread", input => { input.turns[2].external_provider_session_id = "other-thread" }, /thread identity changed/],
  ["missing post-restore state proof", input => { input.browserStateEvidence.afterRestore.turnId = "missing-state-turn" }, /post-restore browser-state proof/],
  ["pre-save action history", input => { input.actionHistory[1].actor_id = "agent:wrong" }, /action history changed/],
  ["post-restore action", input => { input.actionHistory.pop() }, /post-restore Browser action is absent/],
  ["duplicate post-restore Browser action", input => {
    input.actionHistory.push({ ...input.actionHistory.at(-1), action_id: "browser-action-3", sequence: 4 })
  }, /mutate the focused Browser tab exactly once/],
]) {
  test(`save and restore rejects changed ${label}`, () => {
    const input = makeInput()
    mutate(input)
    assert.throws(() => assertRoomSharedBrowserPersistence(input), message)
  })
}

test("physical Browser fixture count proves exactly one fresh post-restore click", () => {
  assert.equal(roomSharedBrowserClickCount("POINTER_CLICK_READY\nPOINTER_CLICK_COUNT=2\n"), 2)
  assert.deepEqual(assertRoomSharedBrowserClickAdvancedOnce(2, 3), { before: 2, after: 3, delta: 1 })
})

test("physical Browser fixture count rejects an unchanged click effect", () => {
  assert.throws(() => assertRoomSharedBrowserClickAdvancedOnce(2, 2), /did not change/)
})

test("physical Browser fixture count rejects a duplicate click effect", () => {
  assert.throws(() => assertRoomSharedBrowserClickAdvancedOnce(2, 4), /more than once/)
})

test("physical Browser fixture count rejects missing or ambiguous markers", () => {
  assert.throws(() => roomSharedBrowserClickCount("POINTER_CLICK_READY"), /exactly one physical click count/)
  assert.throws(() => roomSharedBrowserClickCount("POINTER_CLICK_COUNT=2\nPOINTER_CLICK_COUNT=3"),
    /exactly one physical click count/)
})

function makeInput() {
  const before = roomSharedBrowserReconnectSnapshot({
    sessionId: "room-1",
    environment: {
      session_id: "room-1", environment_id: "environment-1", runtime_generation: 8,
      lifecycle: "ready", focused_tab_id: "tab-1",
      tabs: [{ tab_id: "tab-1", url: "https://fixture.example/click", title: "Browser fixture",
        document_revision: 4, focused: true }],
    },
    session: {
      id: "room-1",
      agents: [{ id: "agent-1", session_id: "room-1", provider: "codex", model: "gpt-6-luna",
        account_profile: "profile-1" }],
    },
    providerAgentId: "agent-1",
    providerEvidence: {
      agentId: "agent-1", provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1",
      mode: "browser", actorId: "agent:agent-1", actionId: "browser-action-1",
    },
    computerActionId: "computer-action-1", computerActorId: "user:web",
    provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1",
    turns: [{ turn_id: "turn-1", lifecycle: "completed", external_provider: "codex",
      external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-1" }],
    actionHistory: [
      { action_id: "browser-action-1", sequence: 1, actor_id: "agent:agent-1", runtime_generation: 8,
        mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }] },
      { action_id: "computer-action-1", sequence: 2, actor_id: "user:web", runtime_generation: 8,
        mode: "computer", kind: "pointer_click", state: "completed", targets: [{ kind: "desktop" }] },
    ],
  })
  const environment = {
    session_id: "room-1", environment_id: "environment-1", runtime_generation: 9,
    lifecycle: "ready", focused_tab_id: "tab-1",
    tabs: [{ tab_id: "tab-1", url: "https://fixture.example/click", title: "Browser fixture",
      document_revision: 5, focused: true }],
  }
  const session = {
    id: "room-1",
    agents: [{ id: "agent-1", session_id: "room-1", provider: "codex", model: "gpt-6-luna",
      account_profile: "profile-1" }],
  }
  const turns = [
    { turn_id: "turn-1", lifecycle: "completed", external_provider: "codex",
      external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-1" },
    { turn_id: "state-turn-before", prompt_id: "state-prompt-before", lifecycle: "completed",
      external_provider: "codex", external_provider_session_id: "thread-1" },
    { turn_id: "state-turn-after", prompt_id: "state-prompt-after", lifecycle: "completed",
      external_provider: "codex", external_provider_session_id: "thread-1" },
    { turn_id: "turn-2", prompt_id: "prompt-2", lifecycle: "completed", external_provider: "codex",
      external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-2" },
  ]
  const actionHistory = [
    { action_id: "browser-action-1", sequence: 1, actor_id: "agent:agent-1", runtime_generation: 8,
      mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }] },
    { action_id: "computer-action-1", sequence: 2, actor_id: "user:web", runtime_generation: 8,
      mode: "computer", kind: "pointer_click", state: "completed", targets: [{ kind: "desktop" }] },
    { action_id: "browser-action-2", sequence: 3, actor_id: "agent:agent-1", runtime_generation: 9,
      mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }] },
  ]
  const continuation = {
    agentId: "agent-1", provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1",
    mode: "browser", actionKind: "click", actionId: "browser-action-2", actionSequence: 3,
    settlement: { turnId: "turn-2", promptId: "prompt-2", lifecycle: "completed", agentIdle: true },
  }
  const browserStateEvidence = {
    beforeSave: { turnId: "state-turn-before" },
    afterRestore: { turnId: "state-turn-after" },
  }
  return { before, environment, session, turns, actionHistory, continuation, browserStateEvidence }
}

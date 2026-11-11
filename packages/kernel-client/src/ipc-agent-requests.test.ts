import assert from "node:assert/strict"
import test from "node:test"

import { abortAgentMoveRequest, hideAgentRequest, listDescendantsRequest, setAgentParentRequest, updateAgentSubstitutesRequest } from "./ipc-agent-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION, type OutsideEvent } from "./kernel-types.js"

test("agent substitute add request carries the chosen account profile", () => {

  assert.deepEqual(
    updateAgentSubstitutesRequest({
      sessionId: "session-1",
      agentId: "agent-1",
      action: {
        Add: {
          provider: "codex",
          model: "gpt-5.4",
          variant: "medium",
          account_profile: "work",
        },
      },
    }),
    {
      UpdateAgentSubstitutes: {
        session_id: "session-1",
        agent_id: "agent-1",
        action: {
          Add: {
            provider: "codex",
            model: "gpt-5.4",
            variant: "medium",
            account_profile: "work",
          },
        },
      },
    },
  )
})

test("agent substitute add request omits account profile for default semantics", () => {
  const request = updateAgentSubstitutesRequest({
    sessionId: "session-1",
    agentId: "agent-1",
    action: { Add: { provider: "codex", model: "gpt-5.4" } },
  })
  const add = (request.UpdateAgentSubstitutes as { action: { Add: Record<string, unknown> } })
    .action.Add
  assert.equal("account_profile" in add, false)
})


test("hiding an agent uses the shared HideAgent request", () => {
  assert.deepEqual(hideAgentRequest("room", "agent", true), { HideAgent: { session_id: "room", agent_id: "agent", hidden: true } })
})

test("moving an agent in the tree names its new parent or none", () => {
  assert.deepEqual(setAgentParentRequest("room", "child", "parent"), { SetAgentParent: { session_id: "room", agent_id: "child", parent_agent_id: "parent" } })
  assert.deepEqual(setAgentParentRequest("room", "child", null), { SetAgentParent: { session_id: "room", agent_id: "child", parent_agent_id: null } })
})

test("the sub-agent tree is read by room and agent id", () => {
  assert.deepEqual(listDescendantsRequest("room", "root"), { ListDescendants: { session_id: "room", agent_id: "root" } })
})

// MP-08 / MP-09 / MP-10 / MP-11: older kernels must never interpret cancellation as Return.
test("Move cancellation requires its allocated protocol", () => {
  assert.throws(() => abortAgentMoveRequest("room", "agent", 637), /protocol 638/)
  assert.deepEqual(abortAgentMoveRequest("room", "agent", 638), {
    MoveAgentToLocal: { session_id: "room", agent_ref: "agent", abort_pending: true },
  })
})

test("an outside interaction event names the room agent a sub-agent decision is shown in", () => {
  const event: OutsideEvent = {
    cursor: 1,
    kernel: "k",
    machine: "m",
    session_id: "room",
    agent_id: "room-agent",
    summary: "Approve Tool?",
    at_ms: 1,
    refs: { interaction_id: "perm-1" },
    type: "interaction",
    value: { state: "pending", shown_in: "room-agent" },
  }
  const closed: OutsideEvent["type"] = "interaction"
  assert.equal(event.type, closed)
  assert.equal(event.type === "interaction" && event.value.shown_in, "room-agent")
  assert.ok(LOCAL_DAEMON_PROTOCOL_VERSION >= 1000)
})

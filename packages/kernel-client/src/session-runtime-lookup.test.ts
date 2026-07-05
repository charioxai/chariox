import assert from "node:assert/strict"
import test from "node:test"

import {
  runtimeProviderRunForAgent,
  sessionActiveInteractionForAgent,
  sessionFocusedInteraction,
} from "./session-runtime-lookup.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"
import type { RuntimeProviderRun } from "./kernel-types.js"

test("session active interaction lookup is scoped to an agent", () => {
  const session = makeSession({
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-2",
      kind: "permission",
      level: "info",
      title: "Approve?",
      message: "Approve?",
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
      requested_at_ms: 1,
    }],
  })

  assert.equal(sessionActiveInteractionForAgent(session, "agent-2")?.id, "interaction-1")
  assert.equal(sessionActiveInteractionForAgent(session, "agent-1"), null)
  assert.equal(sessionActiveInteractionForAgent(session, null), null)
})

test("session focused interaction lookup follows focused agent fallback", () => {
  const session = makeSession({
    focused_agent_id: "agent-2",
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2", agent_ref: "agent-2" })],
    active_interactions: [{
      id: "interaction-2",
      agent_id: "agent-2",
      kind: "permission",
      level: "info",
      title: "Approve?",
      message: "Approve?",
      choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
      requested_at_ms: 1,
    }],
  })

  assert.equal(sessionFocusedInteraction(session)?.id, "interaction-2")
  assert.equal(sessionFocusedInteraction({ ...session, focused_agent_id: "missing-agent" }), null)
  assert.equal(sessionFocusedInteraction({ ...session, focused_agent_id: null }), null)
})

test("runtime provider run lookup requires matching agent ownership", () => {
  const run: RuntimeProviderRun = {
    id: "run-1",
    session_id: "session-1",
    provider: "codex",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    account_profile: "default",
    model: "gpt-5.2",
    variant: null,
    usage_tokens_total: null,
    state: "running",
  }

  assert.equal(runtimeProviderRunForAgent(run, "agent-1")?.id, "run-1")
  assert.equal(runtimeProviderRunForAgent(run, "agent-2"), null)
  assert.equal(runtimeProviderRunForAgent(null, "agent-1"), null)
})

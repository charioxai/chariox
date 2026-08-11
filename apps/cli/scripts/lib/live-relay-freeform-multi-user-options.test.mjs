import assert from "node:assert/strict"
import test from "node:test"

import {
  assertCollaborationAgentSelections,
  collaborationProviderModel,
  collaborationSessionAgentDefaults,
} from "./live-relay-freeform-multi-user-options.mjs"

test("collaboration drills use the requested actual provider for the default session agent", () => {
  assert.deepEqual(
    collaborationSessionAgentDefaults("codex", "gpt-5.6-luna", "medium"),
    { provider: "codex", model: "gpt-5.6-luna", effort: "medium" },
  )
  assert.deepEqual(
    collaborationSessionAgentDefaults("claude-headless", "sonnet", "default"),
    { provider: "claude-headless", model: "sonnet", effort: "default" },
  )
  assert.deepEqual(
    collaborationSessionAgentDefaults("opencode", "opencode/kimi-k2.7-code", "default"),
    { provider: "opencode", model: "opencode/kimi-k2.7-code", effort: "default" },
  )
  assert.equal(collaborationProviderModel("opencode", "kimi-k2.7-code"), "opencode/kimi-k2.7-code")
})

test("collaboration drills verify and report actual agent selections", () => {
  const agents = [
    {
      id: "agent-1",
      owner_user_id: "user-1",
      provider: "codex",
      model: "gpt-5.6-luna",
      effort: "medium",
    },
    {
      id: "agent-2",
      owner_user_id: "user-2",
      provider: "codex",
      model: "gpt-5.6-luna",
      effort: "medium",
    },
  ]

  assert.deepEqual(
    assertCollaborationAgentSelections("codex", "gpt-5.6-luna", "medium", agents),
    agents.map((agent) => ({
      id: agent.id,
      ownerUserId: agent.owner_user_id,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
    })),
  )
  assert.throws(
    () => assertCollaborationAgentSelections("codex", "gpt-5.6-luna", "medium", [
      { ...agents[0], provider: "dev-stub", model: "default" },
      agents[1],
    ]),
    /expected codex\/gpt-5\.6-luna\/medium/,
  )
  assert.deepEqual(
    assertCollaborationAgentSelections("claude-headless", "sonnet", "default", [{
      id: "agent-claude",
      owner_user_id: "user-claude",
      provider: "claude-headless",
      model: "claude/claude-sonnet-5",
      effort: "default",
    }]),
    [{
      id: "agent-claude",
      ownerUserId: "user-claude",
      provider: "claude-headless",
      model: "claude/claude-sonnet-5",
      effort: "default",
    }],
  )
})

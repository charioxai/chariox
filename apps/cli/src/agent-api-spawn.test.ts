import assert from "node:assert/strict"
import test from "node:test"

import { spawnAgent } from "./agent-api.js"
import type { LocalIpcClient } from "./ipc.js"

const agent = {
  id: "agent-0",
  agent_ref: "agent-0",
  session_id: "session-1",
  alias: null,
  provider: "opencode",
  model: "gpt-5.4",
  worktree_id: "worktree-1",
  state: "Idle",
  is_processing: false,
  grid_row: 0,
  grid_col: 0,
  grid_row_span: 1,
  grid_col_span: 1,
  created_at_ms: 0,
  last_activity_at_ms: 0,
}

function clientAnswering(answer: unknown): LocalIpcClient {
  return { send: async () => answer } as unknown as LocalIpcClient
}

const options = { provider: "opencode", alias: "builder" }

test("spawnAgent returns the deadline and the branch the kernel answered with", async () => {
  const spawned = await spawnAgent(
    clientAnswering({ AgentSpawned: { agent, deadline_ms: 1_800_000_000_000, branch: "feature-x" } }),
    "session-1",
    options,
  )

  assert.equal(spawned.agent.id, "agent-0")
  assert.equal(spawned.deadline_ms, 1_800_000_000_000)
  assert.equal(spawned.branch, "feature-x")
})

test("spawnAgent decodes the answer of a kernel that sends neither", async () => {
  const spawned = await spawnAgent(clientAnswering({ AgentSpawned: { agent } }), "session-1", options)

  assert.equal(spawned.agent.id, "agent-0")
  assert.equal(spawned.deadline_ms, undefined)
  assert.equal(spawned.branch, undefined)
})

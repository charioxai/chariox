import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.2",
    worktree_id: "/repo",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [makeAgent()],
    config_state: { version: 0, values: {} },
    ...overrides,
  }
}

function fakeClient(handler: (request: Record<string, unknown>) => Record<string, unknown>) {
  const requests: Record<string, unknown>[] = []
  return {
    requests,
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return handler(request)
      },
    },
  }
}

test("executeShellCommand handles shell-local context mutations", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("set model gpt-5.3"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, { model: "gpt-5.3" })
  const next = applyShellCommandResult(context, result)
  assert.equal(next.model, "gpt-5.3")
})

test("executeShellCommand removes shell-local variables", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    variables: { stale: "agent-1", keep: "session-1" },
  })
  const result = await executeShellCommand(parseShellCommand("unset stale"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.variableRemovals, ["stale"])
  const next = applyShellCommandResult(context, result)
  assert.deepEqual(next.variables, { keep: "session-1" })
})

test("executeShellCommand creates a session and binds assignment", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    resolveExistingDirectory: async () => "/repo/qa",
  })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { s: "session-2" })
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
  })
})

test("executeShellCommand lists agents for current session", async () => {
  const agents = [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })]
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListAgents: { session_id: "session-1" } })
    return { AgentsListed: { agents } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /2 agents/)
  assert.deepEqual((result.data as { agents: AgentInstance[] }).agents, agents)
})

test("executeShellCommand spawns remote agent with worktree placement", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    worktree_id: "/remote/qa",
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "mac-mini",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, {
      SpawnAgent: {
        session_id: "session-1",
        provider: "codex",
        alias: "qa",
        model: "gpt-5.2",
        effort: "low",
        worktree_id: "/remote/qa",
        machine_ref: "mac-mini",
        worktree_placement: null,
      },
    })
    return { AgentSpawned: { agent } }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })
  const result = await executeShellCommand(parseShellCommand("agent spawn qa --machine mac-mini --dir /remote/qa as qa"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.bindings, { qa: "agent-remote" })
  assert.deepEqual(result.contextUpdates, { agentId: "agent-remote" })
})

test("executeShellCommand rejects agent commands without current session", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("agent list"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /no current session/)
})

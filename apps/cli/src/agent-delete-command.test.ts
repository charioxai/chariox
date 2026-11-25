import assert from "node:assert/strict"
import test from "node:test"
import { runAgentDelete } from "./agent-delete-command.js"

const kernel = (agents: Record<string, unknown>[], calls: Record<string, unknown>[] = []) => async (body: Record<string, unknown>) => {
  calls.push(body)
  if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
  if ("ListAgents" in body) return { AgentsListed: { agents } }
  if ("GetSessionState" in body) return { SessionState: { agent_activity: { "agent-1": { status: "idle" } } } }
  if ("ListDescendants" in body) return { AgentTree: { tree: { nodes: [{ agent_id: "kid", alias: "worker", state: "running", depth: 1 }] } } }
  if ("DestroyAgent" in body) return { AgentDestroyed: { agent: {} } }
  return { FinishedAgentsDeleted: { deleted: [], kept: [] } }
}

test("delete is the owner's delete: it lists the whole subtree, running agents included, then destroys", async () => {
  const calls: Record<string, unknown>[] = []
  const agents = [{ id: "agent-1", agent_ref: "a1", alias: "orch12", state: "Idle" }]
  const result = await runAgentDelete("orch12", undefined, kernel(agents, calls)) as { deleted: unknown[] }
  assert.deepEqual(result.deleted, [{ id: "agent-1", alias: "orch12", running: false }, { id: "kid", alias: "worker", running: true }])
  assert.deepEqual(calls.at(-1), { DestroyAgent: { session_id: "room", agent_id: "agent-1" } })
})

test("dry run prints what would go and deletes nothing", async () => {
  const calls: Record<string, unknown>[] = []
  const result = await runAgentDelete("a1", "room", kernel([{ id: "agent-1", agent_ref: "a1", state: "Working" }], calls), { dryRun: true }) as { dry_run: boolean; would_delete: { running: boolean }[] }
  assert.equal(result.dry_run, true)
  assert.equal(result.would_delete[0]!.running, true)
  assert.ok(calls.every(body => !("DestroyAgent" in body)))
})

test("delete refuses an unknown or ambiguous agent before sending anything", async () => {
  const sent: Record<string, unknown>[] = []
  await assert.rejects(runAgentDelete("ghost", undefined, kernel([], sent)), /not found/)
  await assert.rejects(runAgentDelete("twin", undefined, kernel([{ id: "1", alias: "twin" }, { id: "2", alias: "twin" }], sent)), /multiple agents/)
  assert.ok(sent.every(body => !("DestroyAgent" in body)))
})

test("a sub-agent in another Room is deleted through its own Room", async () => {
  const calls: Record<string, unknown>[] = []
  const send = async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: "agent-1", agent_ref: "a1", alias: "parent", state: "Idle" }] } }
    if ("GetSessionState" in body) return { SessionState: { agent_activity: {} } }
    if ("ListDescendants" in body) return { AgentTree: { tree: { nodes: [{ agent_id: "far", alias: "far-child", state: "idle", depth: 1, room_id: "other-room" }] } } }
    return { AgentDestroyed: { agent: {} } }
  }
  await runAgentDelete("far-child", undefined, send)
  assert.deepEqual(calls.at(-1), { DestroyAgent: { session_id: "other-room", agent_id: "far" } })
})

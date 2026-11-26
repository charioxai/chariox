import assert from "node:assert/strict"
import test from "node:test"
import { runAgentParent } from "./agent-parent-command.js"
import { COMMANDS, usageOf } from "./agent-command-table.js"

function kernel(calls: Record<string, unknown>[]) {
  return async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: "agent-1", alias: "lane" }, { id: "agent-2", alias: "successor" }] } }
    return { AgentParentChanged: { agent: { id: "agent-1" } } }
  }
}

test("agent parent moves the agent the first reference names under the second, in one call", async () => {
  const calls: Record<string, unknown>[] = []
  const answer = await runAgentParent("lane", "successor", undefined, kernel(calls))
  assert.deepEqual(calls.at(-1), { SetAgentParent: { session_id: "room", agent_id: "agent-1", parent_agent_id: "agent-2" } })
  assert.deepEqual(answer, { AgentParentChanged: { agent: { id: "agent-1" } } })
})

test("agent parent none makes the agent a room agent", async () => {
  const calls: Record<string, unknown>[] = []
  await runAgentParent("lane", "none", "room", kernel(calls))
  assert.deepEqual(calls.at(-1), { SetAgentParent: { session_id: "room", agent_id: "agent-1", parent_agent_id: null } })
})

test("agent parent refuses an unknown agent before sending anything", async () => {
  const calls: Record<string, unknown>[] = []
  await assert.rejects(runAgentParent("lane", "nobody", undefined, kernel(calls)), /not found/)
  assert.ok(calls.every(call => !("SetAgentParent" in call)))
})

test("agent parent is a registered command with its two references", () => {
  const spec = COMMANDS.find(command => command.name === "agent parent")!
  assert.deepEqual(spec.args, ["AGENT", "PARENT|none"])
  assert.match(usageOf("agent parent"), /agent parent/)
})

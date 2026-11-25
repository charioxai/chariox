import assert from "node:assert/strict"
import test from "node:test"
import { parseAgentResumeArgv, runAgentResume } from "./agent-resume-command.js"

function kernel(calls: Record<string, unknown>[]) {
  return async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: "agent", agent_ref: "a1", alias: "worker" }] } }
    return { AgentConfigUpdated: {} }
  }
}

test("agent resume sends the usage_limit resume to the agent the reference names", async () => {
  const calls: Record<string, unknown>[] = []
  await runAgentResume("worker", false, undefined, kernel(calls))
  const sent = calls.at(-1) as { UpdateAgentConfig: Record<string, unknown> }
  assert.equal(sent.UpdateAgentConfig.session_id, "room")
  assert.equal(sent.UpdateAgentConfig.agent_id, "agent")
  assert.deepEqual(sent.UpdateAgentConfig.usage_limit, { action: "resume_when_back" })
})

test("agent resume cancel takes back the resume or cancels the blocked task", async () => {
  const calls: Record<string, unknown>[] = []
  await runAgentResume("a1", true, "room", kernel(calls))
  const sent = calls.at(-1) as { UpdateAgentConfig: Record<string, unknown> }
  assert.deepEqual(sent.UpdateAgentConfig.usage_limit, { action: "cancel" })
})

test("agent resume refuses an unknown agent before sending anything", async () => {
  const calls: Record<string, unknown>[] = []
  await assert.rejects(runAgentResume("nobody", false, undefined, kernel(calls)), /not found/)
  assert.ok(calls.every(call => !("UpdateAgentConfig" in call)))
})

test("the cancel form parses through the registered command: the word, the agent and the flags", () => {
  assert.deepEqual(parseAgentResumeArgv(["cancel", "worker", "--session", "room"]).positionals, ["worker"])
  const parsed = parseAgentResumeArgv(["cancel", "worker", "--session", "room"])
  assert.equal(parsed.cancel, true)
  assert.equal(parsed.flags.get("--session"), "room")
  assert.equal(parseAgentResumeArgv(["worker"]).cancel, false)
  assert.throws(() => parseAgentResumeArgv(["cancel"]), /usage/i)
})

test("a sub-agent is found below its room agent, by alias or id, and an id with its room is taken as it is", async () => {
  const calls: Record<string, unknown>[] = []
  const send = async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: "root", alias: "lead" }] } }
    if ("ListDescendants" in body) return { AgentTree: { tree: { nodes: [{ agent_id: "child-1", alias: "helper" }, { agent_id: "child-2", alias: "far", room_id: "room-2" }] } } }
    return { AgentConfigUpdated: {} }
  }
  await runAgentResume("helper", false, undefined, send)
  let sent = calls.at(-1) as { UpdateAgentConfig: Record<string, unknown> }
  assert.deepEqual([sent.UpdateAgentConfig.session_id, sent.UpdateAgentConfig.agent_id], ["room", "child-1"])
  await runAgentResume("child-2", true, undefined, send)
  sent = calls.at(-1) as { UpdateAgentConfig: Record<string, unknown> }
  assert.deepEqual([sent.UpdateAgentConfig.session_id, sent.UpdateAgentConfig.agent_id], ["room-2", "child-2"], "a child in another room is addressed in that room")
  await runAgentResume("agent-0123", false, "room", async body => "ListAgents" in body ? { AgentsListed: { agents: [] } } : { AgentConfigUpdated: {} })
})

test("a room agent and a sub-agent of another room that share a name are ambiguous: nothing is sent until the person names the room", async () => {
  const calls: Record<string, unknown>[] = []
  const send = async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room-a" }, { id: "room-b" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: (body.ListAgents as { session_id: string }).session_id === "room-a" ? "a-root-worker" : "b-root", alias: (body.ListAgents as { session_id: string }).session_id === "room-a" ? "worker" : "lead" }] } }
    if ("ListDescendants" in body) return (body.ListDescendants as { session_id: string }).session_id === "room-b" ? { AgentTree: { tree: { nodes: [{ agent_id: "b-child", alias: "worker" }] } } } : { AgentTree: { tree: { nodes: [] } } }
    return { AgentConfigUpdated: {} }
  }
  await assert.rejects(runAgentResume("worker", true, undefined, send), /multiple agents match/)
  assert.ok(calls.every(call => !("UpdateAgentConfig" in call)), "nothing was changed")
  await runAgentResume("worker", true, "room-b", send)
  const sent = calls.at(-1) as { UpdateAgentConfig: Record<string, unknown> }
  assert.deepEqual([sent.UpdateAgentConfig.session_id, sent.UpdateAgentConfig.agent_id], ["room-b", "b-child"])
})

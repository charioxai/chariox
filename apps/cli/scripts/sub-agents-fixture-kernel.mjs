#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 A6/A7 (TUI): a fixture kernel feed for the sub-agents drill. It serves one room with one
// agent and answers ListDescendants from <dir>/tree.json; a change of that file pushes a room-change event, as the
// kernel's own tree change would. Usage: node sub-agents-fixture-kernel.mjs <port> <dir>
import { watch, readFileSync, appendFileSync } from "node:fs"
import path from "node:path"
import { createRequire } from "node:module"

const require = createRequire(new URL("../package.json", import.meta.url))
const { WebSocketServer } = require("ws")
const [port, dir] = [Number(process.argv[2]), process.argv[3]]
const treeFile = path.join(dir, "tree.json")
const unknownLog = path.join(dir, "unknown-requests.log")

const agent = {
  id: "room-agent", alias: "room", provider: "claude", model: "sonnet", effort: null, state: "Idle", is_processing: false, created_at_ms: 1, archived: false,
}
export const session = {
  id: "room1", project_id: "project-default", workspace_id: "/workspace", worktree_id: "/workspace", created_at_ms: 1, status: "Active",
  active_provider_run_id: null, attachment_ids: [], active_prompt: null, queued_prompts: [], focused_agent_id: agent.id, max_agents: 8,
  agents: [agent], config_state: { version: 1, values: {} },
}
const readTree = () => {
  try { return JSON.parse(readFileSync(treeFile, "utf8")) } catch { return { root: agent.id, rev: 0, totals: { live: 0, running: 0, needs_user: 0 }, nodes: [] } }
}
const noWorkflows = { session_id: session.id, home_kernel_id: "fixture-kernel", revision: "0", workflow_count: 0, workflows: [] }
const answers = {
  RelayStatus: () => ({ RelayStatus: { status: { daemon_id: "fixture-kernel", machine_id: "fixture-machine", capabilities: [] } } }),
  ResolveSession: () => ({ SessionResolved: { session } }),
  AttachSessionView: () => ({ SessionViewAttached: {
    attachment: { id: "attachment-1", session_id: session.id },
    state: { SessionState: { session, agent_activity: {}, agent_activity_revision: 0, room_workflows: { session_id: session.id, home_kernel_id: "fixture-kernel", revision: "0", workflow_count: 0, workflows: [] }, processes: [] } },
    history: { SessionHistoryOutline: { agents: [{ agent_id: agent.id, turns: [], next_cursor: null }] } },
  } }),
  GetSessionState: () => ({ SessionState: { session, agent_activity: {}, agent_activity_revision: 0, room_workflows: noWorkflows, processes: [] } }),
  GetSessionHistoryOutline: () => ({ SessionHistoryOutline: { agents: [{ agent_id: agent.id, turns: [], next_cursor: null }] } }),
  AttachToSession: () => ({ SessionAttached: { attachment: { id: "attachment-1", session_id: session.id } } }),
  GetPromptInputHistory: () => ({ PromptInputHistory: { entries: [] } }),
  CloudRelayStatus: () => ({ CloudRelayStatus: { profile: null } }),
  KernelBrowser: () => ({ KernelBrowser: { result: { op: "list_grants", grants: [] } } }),
  GetCredentialVaultStatus: () => ({ CredentialVaultStatus: { status: { path: "/fixture/vault", unlocked: false, expires_at_ms: null }, unlock: {} } }),
  ListDescendants: () => ({ AgentTree: { tree: readTree() } }),
}

const wss = new WebSocketServer({ port, host: "127.0.0.1" })
let eventId = 0
wss.on("connection", socket => socket.on("message", bytes => {
  const frame = JSON.parse(String(bytes))
  if (frame.type !== "request") appendFileSync(path.join(dir, "frames.log"), `${JSON.stringify(frame).slice(0, 400)}\n`)
  const name = frame.request ? Object.keys(frame.request)[0] : frame.type
  if (frame.type === "subscribe") {
    socket.send(JSON.stringify({ type: "event", request_id: frame.request_id, event_id: ++eventId, event: { event: "session_snapshot", room_workflows: noWorkflows, session, provider_run: null, agent_activity: {}, agent_activity_revision: 0 } }))
    return
  }
  const answer = frame.type === "request" ? answers[name]?.(frame.request[name]) : undefined
  if (frame.type === "request") {
    if (!answer) appendFileSync(unknownLog, `${name}\n`)
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: answer ?? {}, error: null }))
  }
}))
watch(dir, (_, file) => {
  if (file !== "tree.json") return
  for (const socket of wss.clients) socket.send(JSON.stringify({ type: "event", event_id: ++eventId, event: { event: "agent_activity_changed", session_id: session.id, agent_activity: {}, agent_activity_revision: eventId } }))
})
console.log("listening", port)

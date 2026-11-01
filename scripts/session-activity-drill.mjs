#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: local 780 / peer 330 semantic activity.
import assert from "node:assert/strict"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
import { LocalIpcClient, LocalIpcError } from "../packages/kernel-client/dist/ipc.js"
import * as request from "../packages/kernel-client/dist/ipc-requests.js"

// Run against an isolated real kernel with a product-linked Codex profile:
// CHARIOX_HOME=<isolated home> CHARIOX_KERNEL_PORT=<port> node scripts/session-activity-drill.mjs <workspace> <profile id>
const [workspaceArg, profile] = process.argv.slice(2)
assert.ok(workspaceArg && profile && process.env.CHARIOX_HOME && process.env.CHARIOX_KERNEL_PORT,
  "Set CHARIOX_HOME and CHARIOX_KERNEL_PORT; pass an external workspace and a linked Codex profile id")
const workspace = resolve(workspaceArg), checkout = resolve(import.meta.dirname, "..")
assert.ok(workspace !== checkout && !workspace.startsWith(checkout + "/"), "Use a workspace outside the checkout")
const home = resolve(process.env.CHARIOX_HOME)
assert.ok(home !== checkout && !home.startsWith(checkout + "/"), "Use a kernel home outside the checkout")
await mkdir(workspace, { recursive: true })
const client = new LocalIpcClient(`ws://127.0.0.1:${process.env.CHARIOX_KERNEL_PORT}`, { localAuthEnvironment: process.env })
const unwrap = (reply, variant) => { assert.ok(reply[variant], `Expected ${variant}`); return reply[variant] }
const snapshot = async () => unwrap(await client.send(request.getWaitingRoomPublicSnapshotRequest()), "WaitingRoomPublicSnapshot").snapshot
const wait = async predicate => {
  const deadline = Date.now() + 120_000
  while (Date.now() < deadline) {
    const value = await snapshot()
    if (predicate(value)) return value
    await new Promise(resolve => setTimeout(resolve, 100))
  }
  throw new Error("The real provider did not reach the expected activity state within two minutes")
}
let sessionId, attachmentId
try {
  const created = unwrap(await client.send(request.createSessionRequest(workspace, workspace, "Session activity", {
    provider: "codex", model: "gpt-6-luna", effort: "low", permission_level: "yolo", account_profile: profile,
  })), "SessionCreated")
  sessionId = created.session.id
  const before = await snapshot(), row = before.sessions.find(session => session.id === sessionId)
  assert.equal(before.schema_version, 14)
  assert.ok(row.last_activity_at_ms > 0)
  const attached = unwrap(await client.send(request.attachToSessionRequest(sessionId, "session-activity-drill")), "SessionAttached")
  attachmentId = attached.attachment.id
  await client.send(request.setRoomPinnedRequest(sessionId, true))
  const pinned = (await snapshot()).sessions.find(session => session.id === sessionId)
  assert.ok(pinned.pinned_at_ms > 0)
  assert.equal(pinned.last_activity_at_ms, row.last_activity_at_ms, "focus and pinning are not semantic activity")
  const ordered = await snapshot()
  await client.send(request.submitPromptRequest(sessionId, attachmentId, created.agent.id,
    'Reply with exactly "Activity verified." Do no other work and call no tools.', []))
  const started = await wait(value => value.sessions.some(session => session.id === sessionId && session.last_activity_at_ms > row.last_activity_at_ms))
  assert.notEqual(started.inventory_version, ordered.inventory_version, "ordering changes invalidate a paging cursor")
  await assert.rejects(client.send({ ListWaitingRoomSessions: { cursor: null, limit: 1, inventory_version: ordered.inventory_version } }),
    error => error instanceof LocalIpcError && error.code === "waiting_room_inventory_changed")
  const completed = await wait(value => value.sessions.some(session => session.id === sessionId
    && session.last_activity_at_ms > row.last_activity_at_ms && session.activity.working_agent_count === 0 && session.activity.active_prompt_count === 0))
  const agent = completed.sessions.find(session => session.id === sessionId).agents.find(agent => agent.id === created.agent.id)
  assert.equal(agent.model, "gpt-6-luna")
  assert.notEqual(agent.activity?.error, true, "the real provider turn must succeed")
  console.log(JSON.stringify({ protocol: 780, peer: 330, schema: 14, provider: "codex", model: agent.model, turns: 1,
    checks: ["semantic activity", "pin insertion timestamp", "focus is not activity", "prompt and turn activity", "paging invalidation"] }))
} finally {
  try {
    if (attachmentId) await client.send(request.detachFromSessionRequest(attachmentId))
    if (sessionId) await client.send(request.deleteSessionRequest(sessionId, workspace))
  } finally { await client.close() }
}

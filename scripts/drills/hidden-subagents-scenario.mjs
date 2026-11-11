// MP-08 / MP-09 / MP-10 / MP-11 A4 (AO-SA-01, AO-SA-09): the `hidden` scenario of orchestrate-one-room.mjs.
// A real orchestrator spawns three leaf workers; the driver then reads what a client sees (agent list, room list,
// snapshot, ListDescendants), prompts a worker by id, deletes the room agent and kills the kernel PROCESS between
// the parent's removal and its children's, and checks that startup recovery ends the rest.
import { execFileSync } from "node:child_process"
import { readdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { listDescendantsRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"

function transcripts(home) {
  const files = []
  const walk = dir => { for (const e of readdirSync(dir, { withFileTypes: true })) { const f = join(dir, e.name); if (e.isDirectory()) walk(f); else if (e.name.endsWith(".jsonl")) files.push(f) } }
  try { walk(join(home, ".claude/projects")) } catch {}
  return files.map(file => readFileSync(file, "utf8"))
}

const WORKERS = ["w1", "w2", "w3", "g1", "g2"]

export function hiddenBrief({ model, work }) {
  return `# Orchestrator brief (drill, hidden scenario)
Use ONLY the chariox tools named below, one call at a time, and do not retry a refused command.
1. With the chariox room_run_command tool spawn three workers: \`agent spawn wN --provider claude --model ${model} --effort low --permission yolo\` for N = 1, 2, 3.
2. Call the chariox room_overview tool with include_events false and include_workflows false and write its exact JSON answer into ${join(work, "overview.txt")} with your own shell.
3. With room_run_command run \`agent list\` and write its exact answer into ${join(work, "agent-list.txt")}; run \`agent children\` and write its exact answer into ${join(work, "children.txt")}.
4. Prompt w1 with the room command \`prompt w1 "<text>"\`: "Run these room commands with the chariox room_run_command tool: \`agent spawn g1 --provider claude --model ${model} --effort low --permission yolo\` and \`agent spawn g2 --provider claude --model ${model} --effort low --permission yolo\`. Report each exact answer. Do not prompt or wait for g1 or g2. End with GDONE."
Do not subscribe to or wait for the workers. Do not delete anything. End your reply with ODONE.
`
}

export async function runHidden(ctx) {
  const { a, R, orch, attached, model, work, note, until, restart, evidence, kill = true } = ctx
  const rpc = async (client, request) => {
    const result = await client.send(request)
    if (result.Error) throw new Error(`${Object.keys(request)[0]}: ${result.Error.code}`)
    return result
  }
  const verdicts = []
  const check = (name, ok, detail = {}) => { verdicts.push({ name, ok }); note(`check ${name}`, { ok, ...detail }) }
  const tree = async (client, agent) => (await rpc(client, listDescendantsRequest(R, agent))).AgentTree.tree
  const exists = async (client, agent) => rpc(client, listDescendantsRequest(R, agent)).then(() => true, () => false)

  await a.rpc(submitPromptRequest(R, attached, orch, `Read ${join(work, "BRIEF.md")} and carry it out now.`, []))
  note("brief-submitted")
  let nodes = []
  await until(async () => (nodes = (await tree(a.client, orch).catch(() => ({ nodes: [] }))).nodes).length === 5, "five sub-agents", 600000)
  const ids = Object.fromEntries(nodes.map(n => [n.alias, n.agent_id]))
  note("descendants", { nodes: nodes.map(n => ({ alias: n.alias, state: n.state, depth: n.depth, outcome: n.outcome })) })
  check("ListDescendants shows the three workers", WORKERS.every(w => ids[w]))
  await until(async () => { const s = await a.rpc({ GetSessionState: { session_id: R } }); return !s.SessionState.session.prompt_states?.[orch]?.active_prompt }, "orchestrator idle", 300000).catch(() => note("orchestrator-still-busy"))

  const listed = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
  const snapshot = (await a.rpc({ GetSessionState: { session_id: R } })).SessionState.session
  const rooms = (await a.rpc({ ListSessions: null })).SessionsListed.sessions
  const room = rooms.find(s => s.id === R)
  check("ListAgents shows only the room agent", listed.length === 1 && listed[0].id === orch, { aliases: listed.map(x => x.alias) })
  check("snapshot shows only the room agent", (snapshot.agents ?? []).length === 1, { count: (snapshot.agents ?? []).length })
  check("room list shows only the room agent", (room?.agents ?? []).length === 1, { count: (room?.agents ?? []).length })
  // max_agents is 2 for this kernel (see the driver config): the room agent plus one peer fill it, five sub-agents do not count.
  const peer1 = await a.rpc({ SpawnAgent: { session_id: R, alias: "peer1", provider: "claude", model, effort: "low", permission_level: "yolo", worktree_id: null } }).then(() => "ok", e => e.message.slice(0, 160))
  const peer2 = await a.rpc({ SpawnAgent: { session_id: R, alias: "peer2", provider: "claude", model, effort: "low", permission_level: "yolo", worktree_id: null } }).then(() => "ok", e => e.message.slice(0, 160))
  check("max_agents ignores the five sub-agents (one peer fits, a second is refused)", peer1 === "ok" && peer2 !== "ok", { peer1, peer2 })
  const read = f => { try { return readFileSync(join(work, f), "utf8") } catch { return null } }
  const overview = read("overview.txt")
  note("room-command-answers", { overview: overview?.slice(0, 1500) ?? null, agent_list: read("agent-list.txt")?.slice(0, 600) ?? null, children: read("children.txt")?.slice(0, 600) ?? null })
  check("room overview counts only the room agent", overview !== null && !WORKERS.some(w => overview.includes(`"alias":"${w}"`) || overview.includes(`"alias": "${w}"`)), { has_text: overview !== null })
  writeFileSync(join(evidence, "views.json"), JSON.stringify({ listed, snapshot_agents: snapshot.agents, room_agents: room?.agents, tree: nodes }, null, 1), { mode: 0o600 })

  // A request by agent id keeps working: the driver prompts a hidden worker as a client addressing it by id.
  const before = Date.now()
  await a.rpc(submitPromptRequest(R, attached, ids.w1, "Reply with exactly HIDDENOK and nothing else.", []))
  const answered = () => transcripts(join(a.dir, "home")).some(file => file.split("\n").some(line => line.includes('"role":"assistant"') && line.includes("HIDDENOK")))
  await until(async () => answered(), "w1 answers", 180000).catch(() => {})
  check("a by-id prompt to a hidden worker runs", answered(), { wait_s: Math.round((Date.now() - before) / 1000) })

  // Keep every worker mid-turn so ending it takes a moment: the window between the parent's removal and the last child's.
  for (const w of WORKERS) await a.rpc(submitPromptRequest(R, attached, ids[w], "Run the shell command `sleep 240` and then reply SLEPT.", [])).catch(e => note("worker-prompt-refused", { w, message: e.message.slice(0, 120) }))
  await delay(15000)
  const kernelPid = a.child.pid
  const descendants = pids => pids.flatMap(p => { try { return readdirSync("/proc").filter(d => /^\d+$/.test(d)).filter(d => { try { return readFileSync(`/proc/${d}/stat`, "utf8").split(") ")[1].split(" ")[1] === String(p) } catch { return false } }).map(Number) } catch { return [] } })
  let family = [kernelPid]
  for (let i = 0; i < 4; i++) family = [...new Set([...family, ...descendants(family)])]
  note("kernel-family", { pids: family })

  const db = join(a.dir, "chariox/state.db")
  const deletions = () => { try { return execFileSync("sqlite3", ["-readonly", db, "select subject_id from durable_state_events where kind='agent.deleted' order by sequence"]).toString().trim().split("\n").filter(Boolean) } catch { return [] } }
  const nameOf = id => id === orch ? "room-agent" : Object.entries(ids).find(([, v]) => v === id)?.[0] ?? id.slice(0, 8)
  const depthOf = Object.fromEntries(nodes.map(n => [n.alias, n.depth]))
  let aliveAtKill = []
  if (!kill) {
    // Plain delete: the parent goes and the cascade ends the whole subtree on its own.
    const t0 = Date.now()
    await a.rpc({ DestroyAgent: { session_id: R, agent_id: orch } }).then(() => note("destroy-answered", { ms: Date.now() - t0 }), e => note("destroy-error", { message: e.message.slice(0, 120) }))
    await until(async () => { for (const w of WORKERS) if (await exists(a.client, ids[w])) return false; return true }, "cascade ends the subtree", 120000).catch(() => {})
    for (const end = Date.now() + 8000; Date.now() < end && deletions().length < WORKERS.length + 1;) await delay(200)
    const order = deletions().map(nameOf)
    const pos = n => order.indexOf(n)
    const parentFirst = order[0] === "room-agent" && pos("g1") > pos("w1") && pos("g2") > pos("w1") && WORKERS.every(w => pos(w) > 0)
    const contiguous = pos("g1") === pos("w1") + 1 && pos("g2") === pos("w1") + 2
    note("deletion-order", { order, depth: depthOf, parent_before_every_descendant: parentFirst, w1_subtree_contiguous_depth_first: contiguous })
    const left = []
    for (const w of WORKERS) if (await exists(a.client, ids[w])) left.push(w)
    check("every sub-agent is gone after the delete", left.length === 0, { left })
    check("every agent is deleted after its parent (parent-first walk, six deletions recorded)", parentFirst, { order })
    const rest = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
    check("the room agent is gone", !rest.some(x => x.id === orch), { agents: rest.map(x => x.alias) })
    note("hidden-summary", { verdicts, mode: "plain" })
    return verdicts
  }
  // Delete the room agent; the poll client sees the parent go and the kernel dies before its children are all gone.
  const deleting = a.rpc({ DestroyAgent: { session_id: R, agent_id: orch } }).then(() => "destroy-answered", e => `destroy-error ${e.message.slice(0, 80)}`)
  // The kernel's journal records the parent's deletion when it is durable; the kill follows at once, while its children are still being ended.
  const durable = () => { try { return Number(execFileSync("sqlite3", ["-readonly", db, `select count(*) from durable_state_events where kind='agent.deleted' and subject_id='${orch}'`]).toString().trim()) > 0 } catch { return false } }
  let parentGoneAt = null
  for (const end = Date.now() + 60000; Date.now() < end;) {
    if (durable()) { parentGoneAt = Date.now(); break }
    await delay(5)
  }
  if (parentGoneAt === null) throw new Error("the parent's deletion never became durable")
  a.child.kill("SIGKILL")
  const killedAt = Date.now()
  await until(() => a.child.exitCode !== null || a.child.signalCode !== null, "kernel killed", 10000)
  note("kernel-killed", { pid: kernelPid, after_parent_durable_ms: killedAt - parentGoneAt })
  const beforeKill = deletions().map(nameOf)
  const beforeKillCount = beforeKill.length
  note("deleted-before-kill", { names: beforeKill, left_for_recovery: WORKERS.filter(w => !beforeKill.includes(w)) })
  note("destroy-result", { result: await Promise.race([deleting, delay(3000).then(() => "no answer (kernel killed)")]) })
  await restart(a)
  // The children that survived the kill are what startup recovery must end.
  const survivors = []
  for (const w of WORKERS) if (await exists(a.client, ids[w])) survivors.push(w)
  aliveAtKill = survivors
  note("after-restart-first-probe", { alive: survivors })
  await until(async () => { for (const w of WORKERS) if (await exists(a.client, ids[w])) return false; return true }, "recovery ends the rest", 120000).catch(() => {})
  const left = []
  for (const w of WORKERS) if (await exists(a.client, ids[w])) left.push(w)
  check("every sub-agent is gone after the restart", left.length === 0, { left })
  const orderAfter = deletions().map(nameOf)
  note("deletion-order", { order: orderAfter, depth: depthOf, window_hit: aliveAtKill.length > 0 || orderAfter.length > beforeKillCount })
  check("the kernel was killed in the middle of the cascade (recovery had sub-agents left to end)", beforeKillCount < WORKERS.length + 1, { deleted_before_kill: beforeKillCount })
  const rest = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
  check("the room agent is gone", !rest.some(x => x.id === orch), { agents: rest.map(x => x.alias) })
  const stragglers = family.filter(p => { try { readFileSync(`/proc/${p}/stat`); return p !== kernelPid } catch { return false } })
  note("provider-processes-left-after-kill", { pids: stragglers })
  for (const p of stragglers) if (Number.isSafeInteger(p) && p > 1) { try { process.kill(p, "SIGTERM") } catch {} }
  note("hidden-summary", { verdicts, survivors_at_first_probe: aliveAtKill, window_hit: aliveAtKill.length > 0 })
  return verdicts
}

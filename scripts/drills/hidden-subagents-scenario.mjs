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

const WORKERS = ["w1", "w2", "w3"]

export function hiddenBrief({ model, work }) {
  return `# Orchestrator brief (drill, hidden scenario)
Use ONLY the chariox tools named below, one call at a time, and do not retry a refused command.
1. With the chariox room_run_command tool spawn three workers: \`agent spawn wN --provider claude --model ${model} --effort low --permission yolo\` for N = 1, 2, 3.
2. Call the chariox room_overview tool with include_events false and include_workflows false and write its exact JSON answer into ${join(work, "overview.txt")} with your own shell.
3. With room_run_command run \`agent list\` and write its exact answer into ${join(work, "agent-list.txt")}; run \`agent children\` and write its exact answer into ${join(work, "children.txt")}.
Do not prompt, subscribe to or wait for the workers. Do not delete anything. End your reply with ODONE.
`
}

export async function runHidden(ctx) {
  const { a, R, orch, attached, model, work, note, until, restart, evidence } = ctx
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
  await until(async () => (nodes = (await tree(a.client, orch).catch(() => ({ nodes: [] }))).nodes).length === 3, "three sub-agents", 600000)
  const ids = Object.fromEntries(nodes.map(n => [n.alias, n.agent_id]))
  note("descendants", { nodes: nodes.map(n => ({ alias: n.alias, state: n.state, depth: n.depth, outcome: n.outcome })) })
  check("ListDescendants shows the three workers", WORKERS.every(w => ids[w]))
  await until(async () => { const s = await a.rpc({ GetSessionState: { session_id: R } }); return !s.SessionState.session.prompt_states?.[orch]?.active_prompt }, "orchestrator idle", 300000).catch(() => note("orchestrator-still-busy"))

  const listed = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
  const archived = (await a.rpc({ ListArchivedAgents: { session_id: R } })).AgentsListed?.agents ?? []
  const snapshot = (await a.rpc({ GetSessionState: { session_id: R } })).SessionState.session
  const rooms = (await a.rpc({ ListSessions: null })).SessionsListed.sessions
  const room = rooms.find(s => s.id === R)
  check("ListAgents shows only the room agent", listed.length === 1 && listed[0].id === orch, { aliases: listed.map(x => x.alias) })
  check("archived list shows no sub-agent", archived.every(x => !WORKERS.includes(x.alias)), { count: archived.length })
  check("snapshot shows only the room agent", (snapshot.agents ?? []).length === 1, { count: (snapshot.agents ?? []).length })
  check("room list shows only the room agent", (room?.agents ?? []).length === 1, { count: (room?.agents ?? []).length })
  const read = f => { try { return readFileSync(join(work, f), "utf8") } catch { return null } }
  const overview = read("overview.txt")
  note("room-command-answers", { overview: overview?.slice(0, 1500) ?? null, agent_list: read("agent-list.txt")?.slice(0, 600) ?? null, children: read("children.txt")?.slice(0, 600) ?? null })
  check("room overview counts only the room agent", overview !== null && !WORKERS.some(w => overview.includes(`"alias":"${w}"`) || overview.includes(`"alias": "${w}"`)), { has_text: overview !== null })
  writeFileSync(join(evidence, "views.json"), JSON.stringify({ listed, archived, snapshot_agents: snapshot.agents, room_agents: room?.agents, tree: nodes }, null, 1), { mode: 0o600 })

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

  // Delete the room agent; the poll client sees the parent go and the kernel dies before its children are all gone.
  const deleting = a.rpc({ DestroyAgent: { session_id: R, agent_id: orch } }).then(() => "destroy-answered", e => `destroy-error ${e.message.slice(0, 80)}`)
  // The kernel's journal records the parent's deletion when it is durable; the kill follows at once, while its children are still being ended.
  const db = join(a.dir, "chariox/state.db")
  const durable = () => { try { return Number(execFileSync("sqlite3", ["-readonly", db, `select count(*) from durable_state_events where kind='agent.deleted' and subject_id='${orch}'`]).toString().trim()) > 0 } catch { return false } }
  let parentGoneAt = null, aliveAtKill = null
  for (const end = Date.now() + 60000; Date.now() < end;) {
    if (durable()) { parentGoneAt = Date.now(); break }
    await delay(5)
  }
  if (parentGoneAt === null) throw new Error("the parent's deletion never became durable")
  a.child.kill("SIGKILL")
  const killedAt = Date.now()
  await until(() => a.child.exitCode !== null || a.child.signalCode !== null, "kernel killed", 10000)
  note("kernel-killed", { pid: kernelPid, after_parent_durable_ms: killedAt - parentGoneAt })
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
  const rest = (await a.rpc({ ListAgents: { session_id: R } })).AgentsListed.agents
  check("the room agent is gone", !rest.some(x => x.id === orch), { agents: rest.map(x => x.alias) })
  const stragglers = family.filter(p => { try { readFileSync(`/proc/${p}/stat`); return p !== kernelPid } catch { return false } })
  note("provider-processes-left-after-kill", { pids: stragglers })
  for (const p of stragglers) if (Number.isSafeInteger(p) && p > 1) { try { process.kill(p, "SIGTERM") } catch {} }
  note("hidden-summary", { verdicts, survivors_at_first_probe: aliveAtKill, window_hit: aliveAtKill.length > 0 })
  return verdicts
}

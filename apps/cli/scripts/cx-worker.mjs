// Drive Chariox Codex workers through their home kernel.
//   node apps/cli/scripts/cx-worker.mjs new <alias> <git-worktree-dir>     create a session (workspace = repo root, worktree = dir) and its Codex agent
//   node apps/cli/scripts/cx-worker.mjs new-remote <alias> <builder-path> create a home session and lease its Codex agent on CX_REMOTE_KERNEL
//   node apps/cli/scripts/cx-worker.mjs say <alias> <prompt-file>          submit a prompt to the alias's agent
//   node apps/cli/scripts/cx-worker.mjs wait <alias> [max-minutes]         block until the agent's current turn finishes (run in background to be notified)
//   node apps/cli/scripts/cx-worker.mjs st                                 status of every worker agent
//   node apps/cli/scripts/cx-worker.mjs out <alias> [chars]                last output of the alias's agent
//   node apps/cli/scripts/cx-worker.mjs drop <alias>                       delete the alias's session
// Set CX_IPC_MODULE to the built CLI ipc.js module if needed.
// CX_SESSION_DB must be an absolute path outside source checkouts.
// Set KERNEL_URL, CX_PROFILE and CX_REMOTE_KERNEL in the operator environment.
// Remote checkout paths belong to the worker; the current local git checkout anchors the home session.
import { readFileSync, writeFileSync, existsSync } from "node:fs"
import { execFileSync } from "node:child_process"
import { isAbsolute } from "node:path"
import { pathToFileURL } from "node:url"
const { LocalIpcClient } = await import(process.env.CX_IPC_MODULE ? pathToFileURL(process.env.CX_IPC_MODULE).href : new URL("../dist/ipc.js", import.meta.url).href)
const DB = process.env.CX_SESSION_DB
if (!DB || !isAbsolute(DB)) throw new Error("CX_SESSION_DB must be an absolute path outside source checkouts")
const REMOTE_KERNEL = process.env.CX_REMOTE_KERNEL
const PROFILE = process.env.CX_PROFILE
const MODEL = process.env.CX_MODEL ?? "gpt-6.1-sol"
if (!process.env.KERNEL_URL) throw new Error("KERNEL_URL must identify the home kernel")
const client = new LocalIpcClient(process.env.KERNEL_URL, {})
const db = existsSync(DB) ? JSON.parse(readFileSync(DB, "utf8")) : {}
const save = () => writeFileSync(DB, JSON.stringify(db, null, 2))
const first = (x, key) => { if (x && typeof x === "object") { if (key in x && typeof x[key] === "string") return x[key]; for (const v of Object.values(x)) { const r = first(v, key); if (r) return r } } return null }
const findAgent = (x, id) => { if (x && typeof x === "object") { if (x.id === id || x.agent_id === id) return x; for (const v of Object.values(x)) { const r = findAgent(v, id); if (r) return r } } return null }
const git = (dir, ...args) => execFileSync("git", ["-C", dir, ...args], { encoding: "utf8" }).trim()
const [cmd, alias, arg] = process.argv.slice(2)

if (cmd === "new" || cmd === "new-remote") {
  if (!alias || !arg) throw new Error(`${cmd} requires an alias and checkout path`)
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(alias)) throw new Error("alias must start with a letter or digit and contain only letters, digits, underscores, or hyphens")
  if (Object.hasOwn(db, alias)) throw new Error(`alias ${alias} already exists`)
  const remote = cmd === "new-remote"
  if (!PROFILE) throw new Error("CX_PROFILE must identify the selected home account")
  if (remote && !REMOTE_KERNEL) throw new Error("CX_REMOTE_KERNEL must identify the worker kernel")
  if (remote && !isAbsolute(arg)) throw new Error("remote checkout path must be absolute")
  const homeCheckout = remote ? process.cwd() : arg
  const worktree = git(homeCheckout, "rev-parse", "--show-toplevel")
  const common = git(homeCheckout, "rev-parse", "--path-format=absolute", "--git-common-dir")
  const workspace = common.endsWith("/.git") ? common.slice(0, -5) : worktree
  const created = await client.send({ CreateSession: { workspace_id: workspace, worktree_id: worktree, alias: `cx-${alias}`, slice_ref: null } })
  const session = first(created, "session_id") ?? first(created, "id")
  if (!session) throw new Error("kernel did not confirm session creation")
  let spawned
  try {
    spawned = await client.send({ SpawnAgent: { session_id: session, provider: "codex", account_profile: PROFILE, alias, model: MODEL, effort: "high", execution_mode: "build", permission_level: "yolo", worktree_id: remote ? arg : null, kernel_ref: remote ? REMOTE_KERNEL : null, slice_ref: null, worktree_placement: null } })
    if (remote && !spawned.AgentSpawned?.agent?.remote_execution?.worker_kernel_id) throw new Error("kernel did not confirm remote placement")
  } catch (error) {
    await client.send({ DeleteSession: { session_ref: session, workspace_id: workspace } }).catch(() => null)
    throw error
  }
  const agent = first(spawned, "agent_id") ?? first(spawned, "id")
  if (!session || !agent) throw new Error("kernel did not confirm session and agent creation")
  db[alias] = { session, agent, workspace, worktree: remote ? arg : worktree, kernel: remote ? REMOTE_KERNEL : null, created: new Date().toISOString() }; save()
  console.log(alias, "session", session, "agent", agent, "worktree", remote ? arg : worktree)
} else if (cmd === "say") {
  const { session, agent } = db[alias]
  const attached = await client.send({ AttachToSession: { session_id: session, client_id: "cx-driver", capability_level: "FullTerminal" } })
  const attachment = first(attached, "id") ?? first(attached, "attachment_id")
  const prompt = readFileSync(arg, "utf8")
  db[alias].lastSay = Date.now(); save()
  const r = await client.send({ SubmitPrompt: { session_id: session, attachment_id: attachment, target_agent_id: agent, prompt, attachments: [] } })
  console.log(alias, "submitted", JSON.stringify(r).slice(0, 120))
} else if (cmd === "wait") {
  // Done when the newest turn started after the last `say` and has a completion time.
  const { session, agent, lastSay = 0 } = db[alias]
  const deadline = Date.now() + Number(arg ?? 240) * 60_000
  let last = null
  while (Date.now() < deadline) {
    const o = await client.send({ GetSessionHistoryOutline: { session_id: session, agent_ids: [agent], latest_prompt_count: 1 } }).catch(() => null)
    const turn = (o?.SessionHistoryOutline?.agents?.[0]?.turns ?? []).at(-1)
    const started = turn?.started_at_ms ?? turn?.prompt_timestamp_ms ?? turn?.blobs?.[0]?.timestamp_ms ?? turn?.entries?.[0]?.entry?.timestamp_ms ?? 0
    const state = !turn ? "no turn" : started < lastSay - 5_000 ? "old turn" : turn.completed_at_ms ? turn.lifecycle ?? "unknown" : "running"
    if (state !== last) { console.log(new Date().toISOString().slice(11, 19), alias, state); last = state }
    if (state === "completed") break
    if (["failed", "cancelled", "unknown"].includes(state)) throw new Error(`turn ${state} for ${alias}`)
    await new Promise((r) => setTimeout(r, 20_000))
  }
  console.log(alias, "wait ended:", last)
  if (last !== "completed") throw new Error(`wait timed out for ${alias}`)
} else if (cmd === "st") {
  for (const [name, { session, agent }] of Object.entries(db)) {
    const listed = await client.send({ ListAgents: { session_id: session } }).catch((e) => ({ error: e.message }))
    const a = findAgent(listed, agent)
    console.log(name.padEnd(18), String(a?.state ?? listed.error ?? "?").padEnd(10), a?.is_processing ? "processing" : "idle      ", a?.model ?? "", session)
  }
} else if (cmd === "out") {
  // The outline groups older history into blobs and returns the newest entries inline; stitch both.
  const { session, agent } = db[alias]
  const outline = await client.send({ GetSessionHistoryOutline: { session_id: session, agent_ids: [agent], latest_prompt_count: 1 } })
  const turn = (outline.SessionHistoryOutline?.agents?.[0]?.turns ?? []).at(-1) ?? {}
  const entries = [], sequences = new Set()
  const collect = (x) => { if (x && typeof x === "object") { if (typeof x.entry_index === "number") sequences.add(x.entry_index); if (typeof x.kind === "string" && typeof x.text === "string") entries.push(x); else for (const v of Object.values(x)) collect(v) } }
  for (const b of turn.blobs ?? []) collect(await client.send({ GetSessionHistoryBlobContent: { session_id: session, agent_id: agent, blob_id: b.blob_id } }).catch(() => null))
  for (const e of turn.entries ?? []) collect(e)
  // The final provider message can live only in the outline summary.
  const summary = turn.summary?.entry
  if (summary?.kind === "provider_output" && !sequences.has(turn.summary.entry_index) && !entries.some((e) => e.kind === summary.kind && e.text === summary.text && e.timestamp_ms === summary.timestamp_ms && e.merge_key === summary.merge_key)) collect(summary)
  let text = "", key = null, tools = 0
  for (const e of entries) {
    if (e.kind === "provider_output") { if (key && e.merge_key !== key) text += "\n"; text += e.text; key = e.merge_key }
    else if (e.kind === "provider_tool") tools++
  }
  console.log(`[${tools} tool entries; turn ${turn.completed_at_ms ? "completed" : "open"}]\n` + text.slice(-Number(arg ?? 2500)))
} else if (cmd === "drop") {
  const { session, agent, workspace } = db[alias]
  await client.send({ DestroyAgent: { session_id: session, agent_id: agent } })
  console.log(JSON.stringify(await client.send({ DeleteSession: { session_ref: session, workspace_id: workspace ?? null } })).slice(0, 200))
  delete db[alias]; save()
} else {
  console.log("usage: new <alias> <dir> | new-remote <alias> <worker-dir> | say <alias> <file> | wait <alias> [min] | st | out <alias> [chars] | drop <alias>")
}
client.close?.(); process.exit(0)

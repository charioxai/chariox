// D1 / D2 / D3 and the revived-sub-agent case on a real kernel with the dev-stub provider (no model account):
// a child's limit cannot pass its running parent's (the refusal names the next step); after the parent's
// limit ended, the owner gives only the child a new limit and the parent's finished cleanup leaves the
// child's new turn running; the finished parent takes a new limit and a prompt again.
// node scripts/drills/agent-limit-extend.mjs --kernel-binary <kernel> --state-root <~/.chariox/dev/task>
//   --workspace-root <outside-checkout-directory> --evidence-dir <evidence>
import { spawn, execFileSync } from "node:child_process"
import { randomBytes } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { tmpdir } from "node:os"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { spawnAgentsRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir"), workspaceRoot = need("--workspace-root")
if (!stateRoot.includes("/.chariox/dev/")) throw new Error("state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
mkdirSync(workspaceRoot, { recursive: true })
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-extend-"))
const root = mkdtempSync(join(stateRoot, "extend-"))
const children = [], clients = [], steps = [], checks = []
const t0 = Date.now()
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), redact(JSON.stringify(steps, null, 1)) + "\n", { mode: 0o600 })
  console.log(`EXT +${Math.round((Date.now() - t0) / 1000)}s ${name} ${redact(JSON.stringify(data)).slice(0, 300)}`)
}
const check = (id, passed, detail) => {
  checks.push({ id, status: passed ? "PASS" : "FAIL", detail })
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2))
  note(id, { passed, detail })
}
const env = { PATH: process.env.PATH, IS_SANDBOX: "1", CHARIOX_ROOM_AGENT_TOOLS: "1", CHARIOX_PROVIDER_DEV_STUB: "1", RUST_LOG: process.env.DRILL_RUST_LOG ?? "warn", SHELL: "/bin/bash", ...(process.env.CHARIOX_CLAUDE_BIN ? { CHARIOX_CLAUDE_BIN: process.env.CHARIOX_CLAUDE_BIN } : {}) }
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(probe, what, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await probe()) return; await delay(500) }
  throw new Error(`timeout: ${what}`)
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  try { await until(() => child.exitCode !== null || child.signalCode !== null, `PID ${child.pid} exits`, 30000) } catch { child.kill("SIGKILL") }
}
async function kernel() {
  const dir = join(root, "k"), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "vault", "home", "tmp", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/vault/vault.json"\nunlock_policy = "kernel_init"\n`)
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join(socketRoot, "k.sock"), CHARIOX_DAEMON_ALIAS: "extend-drill", CHARIOX_MACHINE_ALIAS: "extend-drill-machine",
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  const log = openSync(join(evidence, "kernel.log"), "a", 0o600)
  const child = spawn(binary, [], { env: localEnv, stdio: ["ignore", log, log] })
  children.push(child)
  const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 1000 })
  clients.push(client)
  const rpc = async request => {
    const result = await client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready", 60000)
  return { dir, client, rpc }
}
let k, room, work
async function main() {
  k = await kernel()
  note("kernel-ready", { binary })
  work = mkdtempSync(join(workspaceRoot, "extend-"))
  execFileSync("git", ["init", "-q", "-b", "main", work])
  writeFileSync(join(work, "README.md"), "Agent limit extend drill workspace\n")
  execFileSync("git", ["-C", work, "add", "README.md"])
  execFileSync("git", ["-C", work, "-c", "user.name=drill", "-c", "user.email=drill@example.invalid", "commit", "-q", "-m", "Start drill"])
  const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "extend",
    agent_defaults: { provider: "dev-stub", account_profile: "default", model: "dev-stub", effort: "low", permission_level: "yolo" } } })).SessionCreated
  room = made.session.id
  const boss = made.agent.id
  await k.rpc({ AliasAgent: { session_id: room, agent_id: boss, alias: "boss" } })
  const spawned = await k.rpc(spawnAgentsRequest(room, [{ provider: "dev-stub", alias: "kid", model: "dev-stub" }]))
  const kid = spawned.AgentsSpawned.agents[0]
  await k.rpc({ SetAgentParent: { session_id: room, agent_id: kid.id, parent_agent_id: boss } })
  const attachment = (await k.rpc({ AttachToSession: { session_id: room, client_id: "extend-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  const deadlineOf = async agent => (await k.rpc({ GetAgentDeadline: { session_id: room, agent_id: agent } })).AgentDeadline.deadline
  const activity = async agent => {
    const state = (await k.rpc({ GetSessionState: { session_id: room } })).SessionState
    return { activity: state.agent_activity?.[agent] ?? null, prompt_state: state.session?.prompt_states?.[agent] ?? null, keys: Object.keys(state), session_keys: Object.keys(state.session ?? {}), all_activity: state.agent_activity, session_queued: state.session?.queued_prompts, session_active: state.session?.active_prompt }
  }
  const refused = async request => { try { await k.rpc(request); return "" } catch (error) { return String(error.message) } }

  const limitMs = Date.now() + 60000
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: boss, deadline_ms: limitMs } })
  note("parent-limit-set", { boss, limitMs })
  const later = await refused({ SetAgentDeadline: { session_id: room, agent_id: kid.id, deadline_ms: limitMs + 3600000 } })
  check("D2-a-child-limit-past-its-running-parents-is-refused-with-the-next-step", later.includes("at or before") && later.includes("agent deadline"), { later })
  // The parent's limit reaches the child as its own (inherited) limit; the refused edit leaves it alone.
  const inherited = await deadlineOf(kid.id)
  check("D2-the-refused-edit-changed-nothing", inherited?.state === "armed" && inherited.deadline_ms === limitMs, { inherited })

  await until(async () => { const d = await deadlineOf(boss); return d?.state === "reached" && !!d?.stopped_at_ms }, "the parent's limit ended and its cleanup finished", 120000)
  check("D3-the-parent-limit-ended-its-run", (await deadlineOf(boss))?.state === "reached", {})
  let blocked = await refused(submitPromptRequest(room, attachment, boss, "Reply EXT_DRILL_BLOCKED", []))
  check("D1-a-prompt-to-the-finished-parent-names-the-next-step", blocked.includes("agent deadline"), { blocked })

  // Only the child gets a new limit; the parent's limit stays ended.
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: kid.id, deadline_ms: Date.now() + 600000 } })
  check("D1-the-child-takes-a-new-limit-under-an-ended-parent", (await deadlineOf(kid.id))?.state === "armed" && (await deadlineOf(boss))?.state === "reached", {})
  const accepted = await refused(submitPromptRequest(room, attachment, kid.id, "Reply EXT_DRILL_CHILD_WORK", []))
  check("D1-the-revived-child-takes-a-prompt", accepted === "", { accepted })
  // The stub answers within a second: the revived child's turn runs to its end under the ended parent,
  // and the scheduler's later ticks (every second) leave its limit and its answer alone.
  const history = () => {
    const db = join(k.dir, "chariox/history/operational.db")
    if (!existsSync(db)) return []
    return JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", db, "SELECT sequence, kind, agent_id, substr(event_json,1,400) AS e FROM history_events ORDER BY sequence"], { maxBuffer: 16e6 }).toString() || "[]")
  }
  await until(() => history().some(row => row.agent_id === kid.id && row.kind === "provider_output"), "the revived child's answer", 20000)
  await delay(5000)
  const rows = history().filter(row => row.agent_id === kid.id)
  const kidDeadline = await deadlineOf(kid.id)
  check("P1-the-revived-child-ran-a-turn-under-the-ended-parent-and-kept-its-new-limit",
    rows.some(row => row.kind === "provider_output") && !rows.some(row => /cancel|stopped|ended/i.test(row.kind)) && kidDeadline?.state === "armed" && (await deadlineOf(boss))?.state === "reached",
    { kinds: rows.map(row => row.kind), kidDeadline })

  // The finished parent takes a new limit (none) and then a prompt.
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: boss, deadline_ms: null } })
  const resumed = await refused(submitPromptRequest(room, attachment, boss, "Reply EXT_DRILL_RESUMED", []))
  check("D1-clearing-the-parents-limit-lets-it-take-a-prompt", resumed === "" && (await deadlineOf(boss)) === null, { resumed })
  if (checks.some(c => c.status === "FAIL")) throw new Error("limit extend assertions failed; see verdict.json")
}
try { await main() } catch (error) { note("failure", { message: redact(error.message) }); process.exitCode = 1 }
finally {
  if (k) {
    // What the kernel saw, kept next to the verdict: history events and the kernel's logs.
    try {
      const history = join(k.dir, "chariox/history/operational.db")
      if (existsSync(history)) writeFileSync(join(evidence, "history-events.json"), redact(execFileSync("sqlite3", ["-readonly", "-json", history, "SELECT kind, substr(event_json,1,1500) AS event FROM history_events ORDER BY sequence"], { maxBuffer: 64e6 }).toString()))
    } catch (error) { note("history-capture-failed", { message: redact(error.message) }) }
  }
  if (k && room) { try { await k.rpc({ EndSession: { session_id: room } }) } catch (error) { note("session-cleanup-failed", { message: redact(error.message) }) } }
  for (const client of clients) await client.close().catch(() => {})
  for (const child of [...children].reverse()) await stop(child)
  if (work) rmSync(work, { recursive: true, force: true })
  rmSync(root, { recursive: true, force: true })
  rmSync(socketRoot, { recursive: true, force: true })
  note("cleanup-finished", { root })
}

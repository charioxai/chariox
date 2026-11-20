// TL1 / TL2 / TL3 / TL4 / TL5 on a real kernel with the dev-stub provider (no model account needed):
// a limited agent is warned, then stopped with a notice that names its sub-agents, its sub-agent
// goes on, a refused prompt names the next step, and a new limit lets the same agent run again.
// node scripts/drills/agent-limit-notices.mjs --kernel-binary <kernel> --state-root <~/.chariox/dev/task>
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
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-limits-"))
const root = mkdtempSync(join(stateRoot, "limits-"))
const children = [], clients = [], steps = [], checks = []
const t0 = Date.now()
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), redact(JSON.stringify(steps, null, 1)) + "\n", { mode: 0o600 })
  console.log(`TL +${Math.round((Date.now() - t0) / 1000)}s ${name} ${redact(JSON.stringify(data)).slice(0, 300)}`)
}
const check = (id, passed, detail) => {
  checks.push({ id, status: passed ? "PASS" : "FAIL", detail })
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2))
  note(id, { passed, detail })
}
const env = { PATH: process.env.PATH, IS_SANDBOX: "1", CHARIOX_ROOM_AGENT_TOOLS: "1", SHELL: "/bin/bash", ...(process.env.CHARIOX_CLAUDE_BIN ? { CHARIOX_CLAUDE_BIN: process.env.CHARIOX_CLAUDE_BIN } : {}) }
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
    CHARIOX_DAEMON_SOCKET: join(socketRoot, "k.sock"), CHARIOX_DAEMON_ALIAS: "limits-drill", CHARIOX_MACHINE_ALIAS: "limits-drill-machine",
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
  work = mkdtempSync(join(workspaceRoot, "limits-"))
  execFileSync("git", ["init", "-q", "-b", "main", work])
  writeFileSync(join(work, "README.md"), "Agent limit notices drill workspace\n")
  execFileSync("git", ["-C", work, "add", "README.md"])
  execFileSync("git", ["-C", work, "-c", "user.name=drill", "-c", "user.email=drill@example.invalid", "commit", "-q", "-m", "Start drill"])
  const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "limits",
    agent_defaults: { provider: "dev-stub", account_profile: "default", model: "dev-stub", effort: "low", permission_level: "yolo" } } })).SessionCreated
  room = made.session.id
  const boss = made.agent.id
  await k.rpc({ AliasAgent: { session_id: room, agent_id: boss, alias: "boss" } })
  const spawned = await k.rpc(spawnAgentsRequest(room, [{ provider: "dev-stub", alias: "kid", model: "dev-stub" }]))
  const kid = spawned.AgentsSpawned.agents[0]
  await k.rpc({ SetAgentParent: { session_id: room, agent_id: kid.id, parent_agent_id: boss } })
  const attachment = (await k.rpc({ AttachToSession: { session_id: room, client_id: "limits-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  const notices = () => {
    const history = join(k.dir, "chariox/history/operational.db")
    if (!existsSync(history)) return []
    const rows = JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", history, "SELECT json_extract(event_json,'$.content') AS text FROM history_events WHERE kind='notice' ORDER BY sequence"], { maxBuffer: 16e6 }).toString() || "[]")
    return rows.map(row => String(row.text))
  }
  const deadlineOf = async agent => (await k.rpc({ GetAgentDeadline: { session_id: room, agent_id: agent } })).AgentDeadline.deadline
  const limitMs = Date.now() + 75000
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: boss, deadline_ms: limitMs } })
  note("limit-set", { boss, limitMs })
  await until(() => notices().some(text => text.includes("ends in about 10 minutes")), "the 10 minute warning", 30000)
  check("TL1-ten-minute-warning", true, { notices: notices().filter(text => text.includes("ends in about")) })
  await until(() => notices().some(text => text.includes("ends in about 1 minute")), "the 1 minute warning", 40000)
  check("TL1-one-minute-warning", notices().filter(text => text.includes("ends in about 1 minute")).length === 1, {})
  await until(() => notices().some(text => text.includes("time limit ended at")), "the stop notice", 120000)
  const ended = notices().filter(text => text.includes("time limit ended at"))
  check("TL2-one-stop-notice-naming-what-stopped-and-what-did-not", ended.length === 1 && ended[0].includes("Its sub-agents were not stopped") && ended[0].includes("kid") && ended[0].includes("agent deadline"), { ended })
  const stopped = await deadlineOf(boss)
  check("TL2-the-limit-is-reached-and-stopped", stopped?.state === "reached" && !!stopped?.stopped_at_ms, { stopped })
  check("TL4-the-sub-agent-has-no-limit-and-was-not-stopped", (await deadlineOf(kid.id)) === null, {})
  let refusal = ""
  try { await k.rpc(submitPromptRequest(room, attachment, boss, "Reply LIMIT_DRILL_REFUSED", [])) } catch (error) { refusal = String(error.message) }
  check("TL5-the-refusal-names-the-next-step", refusal.includes("The owner, or its parent") && refusal.includes("agent deadline"), { refusal })
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: boss, deadline_ms: null } })
  let accepted = true
  try { await k.rpc(submitPromptRequest(room, attachment, boss, "Reply LIMIT_DRILL_RESUMED", [])) } catch (error) { accepted = false; note("resume-refused", { message: redact(error.message) }) }
  check("TL3-clearing-the-limit-lets-the-same-agent-take-a-prompt", accepted && (await deadlineOf(boss)) === null, {})
  if (checks.some(c => c.status === "FAIL")) throw new Error("limit notice assertions failed; see verdict.json")
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

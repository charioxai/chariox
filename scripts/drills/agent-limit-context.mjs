// TL3 with a real model: an agent whose time limit ended keeps its context when its limit is extended.
// A real Claude agent is told a word, its limit ends (the Room is told, a prompt is refused with the next
// step), the limit is cleared, and the same agent says the word back.
// node scripts/drills/agent-limit-context.mjs --attach-home <CHARIOX_HOME> --attach-url <ws://127.0.0.1:PORT/kernel>
//   --claude-profile <name> --state-root <~/.chariox/dev/task> --workspace-root <dir> --evidence-dir <evidence>
// (or, without --attach-*, a drill kernel with CLAUDE_TOKEN_FILE as in agent-pages.mjs)
import { spawn, execFileSync } from "node:child_process"
import { randomBytes } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { tmpdir } from "node:os"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir"), workspaceRoot = need("--workspace-root")
const model = "claude-haiku-5-5"
const tokenFile = process.env.CLAUDE_TOKEN_FILE
const attachHome = flags.get("--attach-home"), attachUrl = flags.get("--attach-url")
const claudeProfile = flags.get("--claude-profile") ?? "default"
if (!attachHome && !tokenFile) throw new Error("CLAUDE_TOKEN_FILE is required for Claude product enrollment")
if (!stateRoot.includes("/.chariox/dev/")) throw new Error("state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
mkdirSync(workspaceRoot, { recursive: true })
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-context-"))
const root = mkdtempSync(join(stateRoot, "context-"))
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
async function attachedKernel() {
  const localEnv = { PATH: process.env.PATH, CHARIOX_HOME: resolve(attachHome) }
  const client = new LocalIpcClient(attachUrl, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 15000 })
  clients.push(client)
  const rpc = async request => {
    const result = await client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await rpc({ RelayStatus: null })
  return { dir: resolve(attachHome), client, rpc, attached: true }
}
async function kernel() {
  if (attachHome) return attachedKernel()
  const dir = join(root, "k"), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "vault", "home", "tmp", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/vault/vault.json"\nunlock_policy = "kernel_init"\n`)
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join(socketRoot, "k.sock"), CHARIOX_DAEMON_ALIAS: "pages-drill", CHARIOX_MACHINE_ALIAS: "pages-drill-machine",
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
  note("kernel-ready", { attached: !!k.attached })
  if (!k.attached) {
    const passkey = randomBytes(24).toString("hex")
    await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: passkey } } })
    await k.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  }
  work = mkdtempSync(join(workspaceRoot, "context-"))
  execFileSync("git", ["init", "-q", "-b", "main", work])
  writeFileSync(join(work, "README.md"), "Agent limit context drill workspace\n")
  execFileSync("git", ["-C", work, "add", "README.md"])
  execFileSync("git", ["-C", work, "-c", "user.name=drill", "-c", "user.email=drill@example.invalid", "commit", "-q", "-m", "Start drill"])
  const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "context",
    agent_defaults: { provider: "claude", account_profile: claudeProfile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
  room = made.session.id
  const agent = made.agent.id
  await k.rpc({ LaunchProviderRun: { session_id: room, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: claudeProfile, model, variant: null, native_tui: false } })
  const attachment = (await k.rpc({ AttachToSession: { session_id: room, client_id: "context-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await k.client.subscribeToKernelEvents(room, attachment)
  const history = k.attached ? join(k.dir, "history/operational.db") : join(k.dir, "chariox/history/operational.db")
  const events = kind => {
    if (!existsSync(history)) return []
    const sql = `SELECT json_extract(event_json,'$.content') AS text, json_extract(event_json,'$.metadata') AS meta FROM history_events WHERE kind='${kind}' AND json_extract(event_json,'$.session_id')='${room}' ORDER BY sequence`
    return JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", history, sql], { maxBuffer: 16e6 }).toString() || "[]").map(row => String(row.text ?? ""))
  }
  const word = `LANTERN${randomBytes(3).toString("hex").toUpperCase()}`
  await k.rpc(submitPromptRequest(room, attachment, agent, `Remember this secret word for later: ${word}. Reply with only the word OK.`, []))
  await until(() => events("provider_output").some(text => text.includes("OK")), "the first answer", 120000)
  note("told-the-word", { word })
  const deadline = Date.now() + 8000
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: agent, deadline_ms: deadline } })
  await until(async () => { const d = (await k.rpc({ GetAgentDeadline: { session_id: room, agent_id: agent } })).AgentDeadline.deadline; return d?.stopped_at_ms }, "the limit stops the agent", 60000)
  const notices = events("notice")
  check("TL2-the-room-is-told", notices.filter(text => text.includes("time limit ended at") && text.includes("agent deadline")).length === 1, { notices: notices.filter(text => text.includes("time limit")) })
  let refusal = ""
  try { await k.rpc(submitPromptRequest(room, attachment, agent, "What was the word?", [])) } catch (error) { refusal = String(error.message) }
  check("TL5-the-refusal-names-the-next-step", refusal.includes("The owner, or its parent") && refusal.includes("agent deadline"), { refusal })
  await k.rpc({ SetAgentDeadline: { session_id: room, agent_id: agent, deadline_ms: null } })
  const before = events("provider_output").length
  await k.rpc(submitPromptRequest(room, attachment, agent, "What was the secret word I told you to remember? Reply with only the word.", []))
  await until(() => events("provider_output").slice(before).some(text => text.includes(word)), "the same agent says the word back", 120000)
  check("TL3-the-same-agent-resumes-with-its-context", true, { word, outputs: events("provider_output").slice(before).map(text => text.slice(0, 120)) })
  if (checks.some(c => c.status === "FAIL")) throw new Error("limit context assertions failed; see verdict.json")
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

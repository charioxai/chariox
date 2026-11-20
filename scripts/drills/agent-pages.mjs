// R3: a real provider agent reads a crowded Room through the agent list and the Room overview.
// A Room of 90 agents answers in small pages of compact rows; the agent finds its parent's alias.
// Run against an owned kernel binary, with state and evidence outside the checkout:
// node scripts/drills/agent-pages.mjs --kernel-binary <kernel> --state-root <~/.chariox/dev/task>
//   --workspace-root <outside-checkout-directory> --evidence-dir <evidence> [--agents 90]
// CLAUDE_TOKEN_FILE holds the Claude login, stored in the drill kernel's Vault and never printed.
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
const lanes = Number(flags.get("--agents") ?? 90)
const model = "claude-haiku-5-5"
const tokenFile = process.env.CLAUDE_TOKEN_FILE
if (!tokenFile) throw new Error("CLAUDE_TOKEN_FILE is required for Claude product enrollment")
if (!stateRoot.includes("/.chariox/dev/")) throw new Error("state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
mkdirSync(workspaceRoot, { recursive: true })
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-pages-"))
const root = mkdtempSync(join(stateRoot, "pages-"))
const children = [], clients = [], steps = [], checks = []
const t0 = Date.now()
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), redact(JSON.stringify(steps, null, 1)) + "\n", { mode: 0o600 })
  console.log(`R3 +${Math.round((Date.now() - t0) / 1000)}s ${name} ${redact(JSON.stringify(data)).slice(0, 300)}`)
}
const check = (id, passed, detail) => {
  checks.push({ id, status: passed ? "PASS" : "FAIL", detail })
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2))
  note(id, { passed, detail })
}
const env = { PATH: process.env.PATH, IS_SANDBOX: "1", CHARIOX_ROOM_AGENT_TOOLS: "1", SHELL: "/bin/bash" }
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
  for (const sub of ["chariox", "home", "tmp", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n`)
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
  note("kernel-ready", { binary })
  const passkey = randomBytes(24).toString("hex")
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: passkey } } })
  await k.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  work = mkdtempSync(join(workspaceRoot, "pages-"))
  execFileSync("git", ["init", "-q", "-b", "main", work])
  writeFileSync(join(work, "README.md"), "Agent pages drill workspace\n")
  execFileSync("git", ["-C", work, "add", "README.md"])
  execFileSync("git", ["-C", work, "-c", "user.name=drill", "-c", "user.email=drill@example.invalid", "commit", "-q", "-m", "Start drill"])
  const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "pages",
    agent_defaults: { provider: "claude", account_profile: "default", model, effort: "low", permission_level: "yolo" } } })).SessionCreated
  room = made.session.id
  const boss = made.agent.id
  await k.rpc({ AliasAgent: { session_id: room, agent_id: boss, alias: "boss" } })
  // The scout is the agent that reads the Room; its parent is the boss.
  const scoutSpawn = await k.rpc(spawnAgentsRequest(room, [{ provider: "claude", accountProfile: "default", alias: "scout", model, effort: "low", permissionLevel: "yolo" }]))
  const scout = scoutSpawn.AgentsSpawned.agents[0]
  await k.rpc({ SetAgentParent: { session_id: room, agent_id: scout.id, parent_agent_id: boss } })
  note("scout", { id: scout.id })
  // The crowd: stub agents that only fill the Room, spread over several parents like real lanes.
  const crowd = []
  for (let start = 0; start < lanes; start += 30) {
    const batch = []
    for (let i = start; i < Math.min(start + 30, lanes); i++) batch.push({ provider: "dev-stub", alias: `lane${i}` })
    crowd.push(...(await k.rpc(spawnAgentsRequest(room, batch))).AgentsSpawned.agents)
  }
  note("crowd", { lanes: crowd.length })
  const parents = [boss, scout.id]
  for (const [index, lane] of crowd.entries()) {
    // Two parents keep every team under the 64 sub-agent cap.
    await k.rpc({ SetAgentParent: { session_id: room, agent_id: lane.id, parent_agent_id: parents[index % 2] } })
  }
  await k.rpc({ LaunchProviderRun: { session_id: room, agent_id: scout.id, adapter_key: "claude", provider: "claude", account_profile: "default", model, variant: null, native_tui: false } })
  const attachment = (await k.rpc({ AttachToSession: { session_id: room, client_id: "pages-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await k.client.subscribeToKernelEvents(room, attachment)
  const out = join(work, "pages.json")
  const prompt = `Use only your Chariox tools and one shell command. 1) Call chariox.list_session_agents with no arguments. 2) Call chariox.list_session_agents with {"alias":"lane8"}. 3) Call chariox.room.overview with {"limit":5,"include_workflows":false}. 4) Run one shell command that writes ${out} as JSON with exactly these keys: parent (the "parent" of the "you" row of call 1), total (the "total" of call 1), rows (the number of rows in "agents" of call 1), next_cursor_present (true or false, call 1), keys (the sorted keys of the first row of call 1), filtered (the aliases of call 2), overview_rows (rows in "agents" of call 3), overview_total (the "total" of call 3), overview_has_agent_tasks (true or false). Then reply PAGES-DONE.`
  await k.rpc(submitPromptRequest(room, attachment, scout.id, prompt, []))
  await until(() => existsSync(out), "the scout wrote its report", 240000)
  const report = JSON.parse(readFileSync(out, "utf8"))
  writeFileSync(join(evidence, "report.json"), JSON.stringify(report, null, 2))
  const expectedTotal = lanes + 2
  check("R3a-parent-alias", report.parent === "boss", { parent: report.parent })
  check("R3a-total", report.total === expectedTotal, { total: report.total, expectedTotal })
  check("R3a-page", report.rows >= 1 && report.rows < expectedTotal && report.next_cursor_present === true, { rows: report.rows })
  check("R3a-compact-keys", JSON.stringify(report.keys) === JSON.stringify(["alias", "id", "model", "parent", "provider", "state", "turn"].filter(key => report.keys.includes(key))) && !report.keys.includes("agent_tasks"), { keys: report.keys })
  check("R3a-alias-filter", Array.isArray(report.filtered) && report.filtered.length === 11 && report.filtered.every(alias => alias.startsWith("lane8")), { filtered: report.filtered })
  check("R3b-overview-page", report.overview_rows === 5 && report.overview_total === expectedTotal && report.overview_has_agent_tasks === false, { rows: report.overview_rows, total: report.overview_total })
  // The size the provider received, from the kernel's own record of the tool results.
  const historyDb = join(k.dir, "chariox/history/operational.db")
  const events = JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", historyDb, "SELECT event_json FROM history_events WHERE kind='provider_tool' ORDER BY sequence"], { maxBuffer: 64e6 }).toString() || "[]")
  const sizes = events.map(row => String(row.event_json)).filter(text => text.includes("list_session_agents") || text.includes("room.overview")).map(text => text.length)
  writeFileSync(join(evidence, "tool-event-sizes.json"), JSON.stringify(sizes))
  check("R3a-reply-under-limit", sizes.length > 0 && Math.max(...sizes) < 32 * 1024, { sizes })
  if (checks.some(c => c.status === "FAIL")) throw new Error("R3 assertions failed; see verdict.json")
}
try { await main() } catch (error) { note("failure", { message: redact(error.message) }); process.exitCode = 1 }
finally {
  if (k && room) { try { await k.rpc({ EndSession: { session_id: room } }) } catch (error) { note("session-cleanup-failed", { message: redact(error.message) }) } }
  for (const client of clients) await client.close().catch(() => {})
  for (const child of [...children].reverse()) await stop(child)
  if (work) rmSync(work, { recursive: true, force: true })
  rmSync(root, { recursive: true, force: true })
  rmSync(socketRoot, { recursive: true, force: true })
  note("cleanup-finished", { root })
}

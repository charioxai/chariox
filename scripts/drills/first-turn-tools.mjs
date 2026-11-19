// FT-01 / FT-02: a provider turn never runs without the agent's Chariox tools.
// FT-01: on a kernel busy with --ballast idle Claude runs and a tool-catalog churn, the first turns of
// --agents workers, submitted at once, all call a Chariox tool and succeed.
// FT-02: Claude's Chariox MCP server is pointed at a listener that never answers (CHARIOX_CLAUDE_BIN wrapper,
// MCP_TIMEOUT=5000); the turn must end failed and say why. An old kernel shows it ending as a success.
// node scripts/drills/first-turn-tools.mjs --cell ft01|ft02 --kernel-binary <kernel>
//   --state-root <~/.chariox/dev/task> --workspace-root <outside-checkout> --evidence-dir <outside-checkout>
//   [--agents 8 --ballast 12] (ft01) [--claude-bin /root/.local/bin/claude] (ft02)
// CLAUDE_TOKEN_FILE holds the Claude login, stored in the drill kernel's Vault and never printed.
// Results come from kernel RPCs and events only, never from the kernel's state files.
import { spawn } from "node:child_process"
import { createHash, randomBytes } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { tmpdir } from "node:os"
import { fileURLToPath } from "node:url"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const count = (key, fallback, min) => { const n = Number(flags.get(key) ?? fallback); if (!Number.isSafeInteger(n) || n < min) throw new Error(`${key} must be an integer of at least ${min}`); return n }
const cell = flags.get("--cell")
if (!["ft01", "ft02"].includes(cell)) throw new Error("--cell must be ft01 or ft02")
const binary = need("--kernel-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir"), workspaceRoot = need("--workspace-root")
const agents = cell === "ft01" ? count("--agents", 8, 1) : 1, ballast = cell === "ft01" ? count("--ballast", 12, 0) : 0
const claudeBin = resolve(flags.get("--claude-bin") ?? "/root/.local/bin/claude")
const model = "claude-haiku-5-5"
const tokenFile = process.env.CLAUDE_TOKEN_FILE
if (!tokenFile) throw new Error("CLAUDE_TOKEN_FILE is required for Claude product enrollment")
if (!stateRoot.includes("/.chariox/dev/")) throw new Error("state must be lane-owned")
if ((evidence + "/").startsWith(fileURLToPath(new URL("../../", import.meta.url)))) throw new Error("evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
mkdirSync(workspaceRoot, { recursive: true })
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-ft-"))
const root = mkdtempSync(join(stateRoot, `ft-${cell}-`))
const children = [], clients = [], steps = [], checks = [], workspaces = [], events = [], held = new Set()
const t0 = Date.now()
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const note = (name, data = {}) => {
  steps.push(JSON.parse(redact(JSON.stringify({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data }))))
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 })
  console.log(`${cell.toUpperCase()} +${Math.round((Date.now() - t0) / 1000)}s ${name} ${redact(JSON.stringify(data)).slice(0, 300)}`)
}
function check(id, passed, detail) {
  checks.push({ id, status: passed ? "PASS" : "FAIL", detail: JSON.parse(redact(JSON.stringify(detail ?? null))) })
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2) + "\n")
  note(id, { passed })
}
const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", CHARIOX_ROOM_AGENT_TOOLS: "1", SHELL: "/bin/bash" }
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(check, what, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(500) }
  throw new Error(`timeout: ${what}`)
}
function start(program, extra, name) {
  const log = openSync(join(evidence, `${name}.log`), "a", 0o600)
  const child = spawn(program, [], { env: { ...env, ...extra }, stdio: ["ignore", log, log] })
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("bad pid")
  children.push(child)
  note("process-start", { label: name, pid: child.pid })
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("invalid owned PID")
  child.kill("SIGTERM")
  try { await until(() => child.exitCode !== null || child.signalCode !== null, `PID ${child.pid} exits`, 30000) } catch {
    child.kill("SIGKILL")
    await until(() => child.exitCode !== null || child.signalCode !== null, `PID ${child.pid} exits after kill`, 5000)
  }
  note("process-exit", { pid: child.pid, code: child.exitCode, signal: child.signalCode })
}
// FT-02: the kernel's Chariox MCP server reaches Claude only through --mcp-config; the wrapper points
// it at a listener that takes the connection and never answers, then execs the real Claude.
function writeClaudeWrapper(port) {
  const wrapper = join(root, "claude-wrapper.sh"), log = join(evidence, "wrapper.log")
  const rewrite = `const fs=require("fs");const [src,out,port,log]=process.argv.slice(1);const c=JSON.parse(fs.readFileSync(src,"utf8"));const s=c.mcpServers&&c.mcpServers.chariox;if(s){const u=new URL(s.url);u.host="127.0.0.1:"+port;s.url=u.toString();fs.appendFileSync(log,"rewrote chariox url to 127.0.0.1:"+port+"\\n")}fs.writeFileSync(out,JSON.stringify(c),{mode:0o600})`
  writeFileSync(wrapper, `#!/bin/bash
args=()
while [ $# -gt 0 ]; do
  if [ "$1" = --mcp-config ] && [ $# -gt 1 ]; then
    out=$(mktemp '${join(root, "tmp")}/ft02-mcp-XXXXXX') || exit 1
    node -e '${rewrite}' "$2" "$out" ${port} '${log}' || exit 1
    args+=("$1" "$out"); shift 2; continue
  fi
  args+=("$1"); shift
done
export MCP_TIMEOUT=5000
exec '${claudeBin}' "\${args[@]}"
`, { mode: 0o700 })
  return { wrapper, log }
}
const toolPayload = text => { try { return JSON.parse(text) } catch { try { return JSON.parse(text.slice(text.indexOf("\n") + 1)) } catch { return null } } }
// Every tool call of a turn, from its outline: one blob per call (preview holds `tool`) or an inline entry.
function toolCalls(turn) {
  const calls = []
  for (const blob of turn.blobs) if (blob.kind === "provider_tool") { const tool = toolPayload(blob.preview ?? "")?.tool; if (tool) calls.push({ tool, at: blob.timestamp_ms }) }
  for (const e of turn.entries) if (e.entry.kind === "provider_tool") { const tool = toolPayload(e.entry.text)?.tool; if (tool) calls.push({ tool, at: e.entry.timestamp_ms }) }
  return calls.sort((a, b) => a.at - b.at)
}
function answerText(w) {
  const outputBlobs = new Set(w.turn.blobs.filter(b => b.kind === "provider_output").map(b => b.blob_id))
  return [w.turn.summary?.entry.text, ...w.turn.entries.filter(e => e.entry.kind === "provider_output").map(e => e.entry.text),
    ...w.pages.filter(p => outputBlobs.has(p.blob_id)).flatMap(p => p.entries.filter(e => e.entry.kind === "provider_output").map(e => e.entry.text))].filter(Boolean).join("\n")
}
const prompt = marker => `Call the Chariox tool chariox.room_overview (MCP name mcp__chariox__chariox_room_overview; if it is not loaded, load it first with ToolSearch query "select:mcp__chariox__chariox_room_overview"). Then answer with exactly one line: ${marker} <the room id it reported>. Do nothing else.`

let client, rpc, room, churnLoop, churning = false, deaf
async function readTurns(workers) {
  const outline = (await rpc({ GetSessionHistoryOutline: { session_id: room, agent_ids: workers.map(w => w.id), latest_prompt_count: 3, include_prefetched_content: true } })).SessionHistoryOutline
  for (const w of workers) {
    const agent = outline.agents.find(a => a.agent_id === w.id)
    w.turn = agent?.turns.find(t => t.prompt_id === w.promptId)
    w.pages = agent?.prefetched_blobs ?? []
  }
  return outline
}
async function main() {
  note("binary", { kernel_sha256: createHash("sha256").update(readFileSync(binary)).digest("hex"), cell, agents, ballast })
  const extra = {}
  let wrapperLog
  if (cell === "ft02") {
    deaf = createServer(socket => { held.add(socket); socket.on("error", () => {}); socket.on("close", () => held.delete(socket)) })
    await new Promise(r => deaf.listen(0, "127.0.0.1", r))
    const written = writeClaudeWrapper(deaf.address().port)
    extra.CHARIOX_CLAUDE_BIN = written.wrapper
    wrapperLog = written.log
    note("deaf-listener", { port: deaf.address().port, claude_bin: claudeBin })
  }
  const ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "logs", "history", "config", "cache", "state"]) mkdirSync(join(root, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(root, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[workflow]\nsession_default_max_agents = ${agents + ballast + 4}\n[state]\npath = "${root}/chariox/state.db"\n`, { mode: 0o600 })
  const kernelEnv = { ...env, HOME: join(root, "home"), TMPDIR: join(root, "tmp"), XDG_CONFIG_HOME: join(root, "config"), XDG_CACHE_HOME: join(root, "cache"), XDG_STATE_HOME: join(root, "state"),
    CHARIOX_HOME: join(root, "chariox"), CHARIOX_LOG_DIR: join(root, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(root, "history"),
    CHARIOX_DAEMON_SOCKET: join(socketRoot, "kernel.sock"), CHARIOX_DAEMON_ALIAS: `ft-${cell}`, CHARIOX_MACHINE_ALIAS: `ft-${cell}-machine`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]), ...extra }
  const work = mkdtempSync(join(workspaceRoot, `ft-${cell}-`))
  workspaces.push(work)
  start(binary, kernelEnv, "kernel")
  client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000 })
  clients.push(client)
  rpc = async request => {
    const result = await client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready", 60000)
  await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: randomBytes(24).toString("hex") } } })
  await rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  const defaults = { provider: "claude", account_profile: "default", model, effort: "low", permission_level: "yolo" }
  const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: `ft-${cell}`, agent_defaults: defaults } })).SessionCreated
  room = made.session.id
  const attachment = (await rpc({ AttachToSession: { session_id: room, client_id: "ft-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  client.onKernelEvent(event => { if (events.length < 20000) events.push(redact(JSON.stringify(event)).slice(0, 4000)) })
  await client.subscribeToKernelEvents(room, attachment)
  const spawnAgent = async alias => (await rpc({ SpawnAgent: { session_id: room, alias, ...defaults } })).AgentSpawned.agent.id
  const ballastIds = []
  for (let i = 0; i < ballast; i++) {
    const id = await spawnAgent(`ballast-${i}`)
    await rpc({ LaunchProviderRun: { session_id: room, agent_id: id, adapter_key: "claude", provider: "claude", account_profile: "default", model, variant: null, native_tui: false } })
    ballastIds.push(id)
  }
  note("room-ready", { room, ballast: ballastIds.length })
  // Revoking a script grant always fires the kernel's tool-catalog change hint, which recomputes
  // every running run's catalog; it needs no registered script.
  const churn = { changes: 0, error: null }
  if (cell === "ft01") {
    churning = true
    const targets = ballastIds.length ? ballastIds : [made.agent.id]
    churnLoop = (async () => {
      while (churning) {
        try { await rpc({ RevokeAgentExtension: { agent_ref: targets[churn.changes % targets.length], kind: "script", name: "ft-churn" } }); churn.changes++ } catch (error) { churn.error = redact(error.message); return }
        await delay(200)
      }
    })()
  }
  const workers = []
  for (let i = 0; i < agents; i++) workers.push({ i, id: await spawnAgent(`worker-${i}`), marker: `${cell.toUpperCase()}-OK-${i}` })
  await Promise.all(workers.map(async w => {
    w.submittedAt = Date.now()
    const outcome = (await rpc(submitPromptRequest(room, attachment, w.id, prompt(w.marker), []))).PromptSubmitted.outcome
    w.promptId = Object.values(outcome)[0].prompt.id
  }))
  note("prompts-submitted", { workers: workers.map(w => ({ agent: w.id, prompt: w.promptId })) })
  const deadline = Date.now() + 6 * 60000
  let outline
  while (Date.now() < deadline) {
    outline = await readTurns(workers)
    if (workers.every(w => w.turn && w.turn.lifecycle !== "open")) break
    await delay(2000)
  }
  churning = false
  await churnLoop
  note("settled", { churn, open: workers.filter(w => !w.turn || w.turn.lifecycle === "open").map(w => w.id) })
  writeFileSync(join(evidence, "history.json"), redact(JSON.stringify(outline, null, 1)), { mode: 0o600 })
  const rows = workers.map(w => {
    const calls = w.turn ? toolCalls(w.turn) : [], chariox = calls.filter(c => c.tool.startsWith("mcp__chariox__"))
    const text = w.turn ? answerText(w) : ""
    return { w, chariox, row: { agent: w.id, lifecycle: w.turn?.lifecycle ?? "no turn", outcome: w.turn?.outcome ?? null, settle_ms: w.turn?.completed_at_ms ? w.turn.completed_at_ms - w.submittedAt : null,
      first_chariox_call_ms: chariox[0] ? chariox[0].at - w.submittedAt : null, tools: calls.map(c => c.tool), marker: text.includes(w.marker), room_id_reported: text.includes(room), answer: text.slice(0, 300) } }
  })
  if (cell === "ft01") {
    for (const { w, chariox, row } of rows) check(`FT-01-worker-${w.i}`, row.lifecycle === "completed" && row.outcome?.kind === "completed" && row.marker && chariox.length > 0, row)
    check("FT-01-churn", churn.changes > 0 && !churn.error, churn)
  } else {
    const { row } = rows[0]
    check("FT-02-wrapper-used", existsSync(wrapperLog) && readFileSync(wrapperLog, "utf8").includes("rewrote chariox url"), { log: existsSync(wrapperLog) ? readFileSync(wrapperLog, "utf8").slice(0, 300) : null })
    check("FT-02-turn-failed", row.lifecycle === "failed", row)
    const reason = row.outcome?.reason ?? ""
    check("FT-02-failure-reason", reason.includes("could not connect to this kernel's Chariox tools") && row.outcome?.retryable === true, { outcome: row.outcome })
  }
}
try { await main() } catch (error) { check("drill-completed", false, { message: redact(error.message) }); process.exitCode = 1 }
finally {
  churning = false
  await churnLoop
  if (rpc && room) { try { await rpc({ EndSession: { session_id: room } }) } catch (error) { note("session-cleanup-failed", { message: redact(error.message) }) } }
  writeFileSync(join(evidence, "events.jsonl"), events.join("\n") + "\n", { mode: 0o600 })
  for (const c of clients) await c.close().catch(() => {})
  for (const child of [...children].reverse()) await stop(child)
  for (const socket of held) socket.destroy()
  if (deaf) await new Promise(r => deaf.close(r))
  for (const workspace of workspaces) rmSync(workspace, { recursive: true, force: true })
  rmSync(root, { recursive: true, force: true })
  rmSync(socketRoot, { recursive: true, force: true })
  writeFileSync(join(evidence, "cleanup.json"), JSON.stringify({ processes: children.map(c => ({ pid: c.pid, exit_code: c.exitCode, signal: c.signalCode })), state_removed: !existsSync(root), workspaces_removed: workspaces.every(w => !existsSync(w)), socket_directory_removed: !existsSync(socketRoot) }, null, 2))
  note("cleanup-finished", { root })
  if (checks.some(c => c.status === "FAIL")) process.exitCode = 1
}

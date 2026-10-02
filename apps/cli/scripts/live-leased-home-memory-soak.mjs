#!/usr/bin/env node
// Home-kernel memory soak for leased agents: relay + home + worker kernels, N leased Claude
// agents backed by a fake streaming CLI, TUIs that poll terminal output, outline polls, idle
// sessions, and the cx-driver pattern that queues prompts while a turn runs. Fails when the home
// kernel's RSS exceeds --max-rss-mb.
import { execFileSync, spawn, spawnSync } from "node:child_process"
import { randomUUID } from "node:crypto"
import { chmodSync, cpSync, mkdirSync, readFileSync, readlinkSync, realpathSync, rmSync, writeFileSync } from "node:fs"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..")
const args = process.argv.slice(2)
const argValue = (name) => { const index = args.indexOf(name); return index >= 0 ? args[index + 1] : undefined }
const argNumber = (name, fallback) => Number(argValue(name) ?? fallback)
if (args.includes("--help")) {
  console.log("Usage: live-leased-home-memory-soak.mjs [--build-profile debug|release] [--agents 12] [--minutes 8] [--prompt-every-ms 3000] [--idle-sessions 35] [--max-rss-mb 1200] [--evidence-dir PATH] [--heaptrack-worker] [--kernel-bin PATH] [--worker-kernel-bin PATH]")
  process.exit(0)
}
const targetDir = path.join(process.env.CARGO_TARGET_DIR ?? path.join(repoRoot, "target"), argValue("--build-profile") ?? "debug")
const evidenceDir = argValue("--evidence-dir")
const heaptrackWorker = args.includes("--heaptrack-worker")
const agents = argNumber("--agents", 12)
const minutes = argNumber("--minutes", 8)
const promptEveryMs = argNumber("--prompt-every-ms", 3_000)
const idleSessions = argNumber("--idle-sessions", 35)
const maxRssMb = argNumber("--max-rss-mb", 1_200)
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const root = path.join(os.tmpdir(), `chariox-leased-home-memory-soak-${process.pid}-${Date.now()}`)
const children = []

// Claude Code stand-in: every prompt starts a long turn of thinking, text and tool deltas.
const fakeClaude = `#!${process.execPath}
const args = process.argv.slice(2)
if (args.includes("--version")) { console.log("2.1.207 (Claude Code)"); process.exit(0) }
if (args[0] === "auth") { console.log(JSON.stringify({ loggedIn: true, authMethod: "claude.ai", email: "soak@example.test", subscriptionType: "pro" })); process.exit(0) }
if (!args.includes("-p")) process.exit(0)
const sid = require("node:crypto").randomUUID()
const out = (value) => process.stdout.write(JSON.stringify(value) + "\\n")
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const text = (n, tag) => (tag + " " + "lorem ipsum dolor sit amet ".repeat(Math.ceil(n / 27))).slice(0, n)
let turns = Promise.resolve(), turn = 0
async function run() {
  turn++
  out({ type: "system", subtype: "init", session_id: sid, model: "claude-soak" })
  for (let message = 1; message <= 150; message++) {
    const id = "msg_" + turn + "_" + message
    out({ type: "stream_event", event: { type: "message_start", message: { id, model: "claude-soak" } }, session_id: sid })
    out({ type: "stream_event", event: { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "" } }, session_id: sid })
    let thinking = "", body = ""
    for (let i = 0; i < 20; i++) { const delta = text(200, "t" + i); thinking += delta; out({ type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: delta } }, session_id: sid }); await sleep(50) }
    out({ type: "stream_event", event: { type: "content_block_start", index: 1, content_block: { type: "text", text: "" } }, session_id: sid })
    for (let i = 0; i < 60; i++) { const delta = text(200, "x" + i); body += delta; out({ type: "stream_event", event: { type: "content_block_delta", index: 1, delta: { type: "text_delta", text: delta } }, session_id: sid }); await sleep(50) }
    out({ type: "assistant", message: { id, model: "claude-soak", content: [{ type: "thinking", thinking }, { type: "text", text: body }, { type: "tool_use", id: "toolu_" + id, name: "Bash", input: { command: "cargo test" } }] }, session_id: sid })
    out({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "toolu_" + id, content: text(4000, "tool-output") }] }, session_id: sid })
  }
  out({ type: "result", subtype: "success", is_error: false, result: "done", session_id: sid })
}
require("node:readline").createInterface({ input: process.stdin })
  .on("line", (line) => { if (line.trim()) turns = turns.then(run) })
  .on("close", () => process.exit(0))
`

class Kernel {
  constructor(port) { this.port = port; this.pending = new Map() }
  async open() {
    this.ws = new WebSocket(`ws://127.0.0.1:${this.port}/kernel`)
    await new Promise((resolve, reject) => { this.ws.onopen = resolve; this.ws.onerror = reject })
    this.ws.onmessage = (message) => {
      const frame = JSON.parse(String(message.data))
      const pending = this.pending.get(frame.request_id)
      if (!pending) return
      this.pending.delete(frame.request_id)
      frame.error ? pending.reject(new Error(JSON.stringify(frame.error))) : pending.resolve(frame.response)
    }
    return this
  }
  frame(frame) { return new Promise((resolve, reject) => { this.pending.set(frame.request_id, { resolve, reject }); this.ws.send(JSON.stringify(frame)) }) }
  send(request) { const id = randomUUID(); return this.frame({ type: "request", request_id: id, command_id: id, request }) }
  subscribe(session_id, attachment_id) { return this.frame({ type: "subscribe", request_id: randomUUID(), session_id, attachment_id }) }
  close() { try { this.ws.close() } catch {} }
}

async function freePort() {
  for (;;) {
    const base = 40_000 + Math.floor(Math.random() * 20_000)
    const free = await Promise.all([0, 10, 11, 12, 13, 20, 21, 22, 23].map((offset) => new Promise((resolve) => {
      const server = net.createServer().once("error", () => resolve(false)).listen(base + offset, "127.0.0.1", () => server.close(() => resolve(true)))
    })))
    if (free.every(Boolean)) return base
  }
}

function startKernel(name, port, base, relayToken, acceptLeases) {
  const home = path.join(root, name)
  mkdirSync(path.join(home, "home"), { recursive: true })
  if (name === "home") writeFileSync(path.join(home, "home", "config.toml"), "[credential_vault]\nbackend = \"process_memory\"\n")
  const kernelBin = (name === "worker" ? argValue("--worker-kernel-bin") : undefined) ?? argValue("--kernel-bin") ?? path.join(targetDir, "chariox-kernel")
  const profileWorker = heaptrackWorker && name === "worker"
  const child = spawn(profileWorker ? "heaptrack" : kernelBin, profileWorker ? ["--record-only", "-o", path.join(evidenceDir, "worker-heaptrack"), kernelBin] : [], {
    cwd: path.join(root, "workspace"),
    detached: process.platform !== "win32",
    stdio: "ignore",
    env: {
      ...process.env,
      HOME: home,
      CHARIOX_HOME: path.join(home, "home"),
      CHARIOX_LOG_DIR: path.join(home, "logs"),
      CHARIOX_DAEMON_ID: `soak-${name}-${process.pid}`,
      CHARIOX_DAEMON_ALIAS: `soak-${name}`,
      CHARIOX_MACHINE_ID: `soak-${name}-machine-${process.pid}`,
      CHARIOX_KERNEL_HOST: "127.0.0.1",
      CHARIOX_KERNEL_PORT: String(port),
      CHARIOX_MCP_PORT: String(port + 1),
      CHARIOX_OPENCODE_PORT: String(port + 2),
      CHARIOX_CODEX_PORT: String(port + 3),
      CHARIOX_DAEMON_SOCKET: path.join(home, "kernel.sock"),
      CHARIOX_RELAY_URL: `ws://127.0.0.1:${base}`,
      CHARIOX_RELAY_TOKEN: relayToken,
      CHARIOX_ACCEPT_REMOTE_LEASES: acceptLeases ? "true" : "false",
      CHARIOX_REMOTE_LEASE_CAPACITY: "64",
      CHARIOX_CLAUDE_BIN: path.join(root, "claude"),
      CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: "1",
      XDG_CONFIG_HOME: path.join(home, "xdg-config"),
      XDG_STATE_HOME: path.join(home, "xdg-state"),
      XDG_CACHE_HOME: path.join(home, "xdg-cache"),
    },
  })
  child.profileKernelBin = profileWorker ? realpathSync(kernelBin) : null
  children.push(child)
  return child
}

async function connect(port) {
  for (let attempt = 0; attempt < 240; attempt++) {
    try { return await new Kernel(port).open() } catch { await sleep(500) }
  }
  throw new Error(`kernel on ${port} did not start`)
}

function rssMb(pid) {
  try {
    if (process.platform === "linux") return Number(/VmRSS:\s+(\d+)/.exec(readFileSync(`/proc/${pid}/status`, "utf8"))[1]) / 1024
    return Number(execFileSync("ps", ["-o", "rss=", "-p", String(pid)], { encoding: "utf8" }).trim()) / 1024
  } catch {
    return NaN
  }
}

const first = (value, key) => {
  if (value && typeof value === "object") {
    if (typeof value[key] === "string") return value[key]
    for (const child of Object.values(value)) { const found = first(child, key); if (found) return found }
  }
  return null
}

let report
try {
  if (heaptrackWorker && !evidenceDir) throw new Error("--heaptrack-worker requires --evidence-dir")
  if (evidenceDir) mkdirSync(evidenceDir, { recursive: true })
  mkdirSync(path.join(root, "workspace"), { recursive: true })
  spawnSync("git", ["init", "-q"], { cwd: path.join(root, "workspace") })
  writeFileSync(path.join(root, "claude"), fakeClaude)
  chmodSync(path.join(root, "claude"), 0o700)
  const base = await freePort()
  const relayToken = randomUUID()
  children.push(spawn(path.join(targetDir, "chariox-relay"), [], { detached: process.platform !== "win32", stdio: "ignore", env: { ...process.env, CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(base), CHARIOX_RELAY_TOKEN: relayToken } }))
  await sleep(1_000)
  startKernel("worker", base + 10, base, relayToken, true)
  const home = startKernel("home", base + 20, base, relayToken, false)
  const control = await connect(base + 20)
  const worker = await connect(base + 10)
  await control.send({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: `soak-token-${randomUUID()}`, overwrite: true } })
  await control.send({ GetProviderAuthStatus: { provider: "claude", account_profile: "default" } })
  const workspace = path.join(root, "workspace")
  const sessions = []
  for (let index = 0; index < agents; index++) {
    const session = first(await control.send({ CreateSession: { workspace_id: workspace, worktree_id: workspace, alias: `soak-${index}`, slice_ref: null } }), "session_id")
    const spawned = await control.send({ SpawnAgent: { session_id: session, provider: "claude", alias: `soak-${index}`, model: "sonnet", effort: "low", execution_mode: "build", permission_level: "yolo", worktree_id: workspace, kernel_ref: `soak-worker-${process.pid}`, slice_ref: null, worktree_placement: null } })
    if (index === 0) {
      // The worker validates the replicated account before its first leased launch.
      const profiles = await worker.send({ ListProviderAccountProfiles: { provider: "claude" } })
      for (const profile of profiles.ProviderAccountProfilesListed?.profiles ?? []) {
        await worker.send({ GetProviderAuthStatus: { provider: "claude", account_profile: profile.profile_id } })
      }
    }
    const viewer = await new Kernel(base + 20).open()
    const attachment = first(await viewer.send({ AttachToSession: { session_id: session, client_id: "soak-tui", capability_level: "FullTerminal" } }), "id")
    await viewer.subscribe(session, attachment)
    sessions.push({ session, agent: spawned.AgentSpawned.agent.id, viewer, attachment })
  }
  for (let index = 0; index < idleSessions; index++) {
    await control.send({ CreateSession: { workspace_id: workspace, worktree_id: workspace, alias: `soak-idle-${index}`, slice_ref: null } })
  }
  const samples = []
  const pumpDurations = []
  let pumpErrors = 0
  const deadline = Date.now() + minutes * 60_000
  let lastPrompt = 0, nextSample = 0, prompts = 0, tick = 0
  while (Date.now() < deadline) {
    for (const { session, agent, viewer, attachment } of sessions) {
      // TUIs poll terminal output; `cx wait` polls the agent's latest turn outline.
      const pumpStarted = performance.now()
      viewer.send({ PumpTerminalOutput: { session_id: session, attachment_id: attachment } })
        .then(() => pumpDurations.push(performance.now() - pumpStarted)).catch(() => { pumpErrors++ })
      if (tick % 20 === 0) control.send({ GetSessionHistoryOutline: { session_id: session, agent_ids: [agent], latest_prompt_count: 1 } }).catch(() => {})
    }
    tick++
    if (Date.now() - lastPrompt >= promptEveryMs) {
      lastPrompt = Date.now()
      for (const { session, agent } of sessions) {
        // cx-driver: a one-shot client submits while the turn runs, then disconnects.
        const driver = await new Kernel(base + 20).open()
        try {
          const attachment = first(await driver.send({ AttachToSession: { session_id: session, client_id: "cx-driver", capability_level: "FullTerminal" } }), "id")
          await driver.send({ SubmitPrompt: { session_id: session, attachment_id: attachment, target_agent_id: agent, prompt: `continue ${prompts}`, attachments: [] } })
          prompts++
        } catch {}
        driver.close()
      }
    }
    if (Date.now() >= nextSample) {
      nextSample = Date.now() + 30_000
      samples.push({ elapsed_s: Math.round((Date.now() + minutes * 60_000 - deadline) / 1000), rss_mb: Math.round(rssMb(home.pid)) })
      console.log(JSON.stringify(samples.at(-1)))
    }
    await sleep(1_000)
  }
  const measured = samples.map((sample) => sample.rss_mb).filter(Number.isFinite)
  const maxObservedMb = measured.length ? Math.max(...measured) : null
  // A run without RSS samples or accepted prompts measured nothing.
  pumpDurations.sort((a, b) => a - b)
  const percentile = (p) => Math.round(pumpDurations[Math.min(pumpDurations.length - 1, Math.floor(p * pumpDurations.length))] ?? 0)
  report = { terminal_pump_errors: pumpErrors, terminal_pump_roundtrip_ms: { count: pumpDurations.length, p50: percentile(0.50), p95: percentile(0.95), max: Math.round(pumpDurations.at(-1) ?? 0) }, agents, idle_sessions: idleSessions, minutes, accepted_prompts: prompts, max_rss_mb: maxObservedMb, limit_mb: maxRssMb, samples, ok: measured.length > 0 && prompts > 0 && maxObservedMb <= maxRssMb }
} finally {
  for (const child of children.reverse()) {
    try {
      if (child.profileKernelBin) {
        // Stop the debuggee first so heaptrack can finish its compressed stream.
        const pids = readFileSync(`/proc/${child.pid}/task/${child.pid}/children`, "utf8").trim().split(/\s+/).filter(Boolean)
        for (const pid of pids) {
          if (readlinkSync(`/proc/${pid}/exe`) === child.profileKernelBin) process.kill(Number(pid), "SIGTERM")
        }
      } else child.kill("SIGTERM")
    } catch {}
  }
  await sleep(3_000)
  for (const child of children) {
    try { process.platform === "win32" ? child.kill("SIGKILL") : process.kill(-child.pid, "SIGKILL") } catch {}
  }
  if (evidenceDir) {
    for (const name of ["home", "worker"]) {
      const logs = path.join(root, name, "logs")
      try { cpSync(logs, path.join(evidenceDir, `${name}-logs`), { recursive: true }) } catch {}
    }
    if (report) writeFileSync(path.join(evidenceDir, "report.json"), JSON.stringify(report, null, 2) + "\n")
  }
  rmSync(root, { recursive: true, force: true })
}
console.log(JSON.stringify(report, null, 2))
process.exit(report.ok ? 0 : 1)

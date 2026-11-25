// A lane-owned drill kernel for the live Vault cells: own CHARIOX_HOME, ports and HOME under /root/.chariox/dev/, a Claude account installed
// through SetProviderAccountCredential from a token FILE (never printed, never in argv), the Vault created and unlocked, one Room with one agent.
// Run this inside a host unit (systemd-run -p MemoryMax=...), not inside an agent shell: the kernel browser needs a host that is not in Bubblewrap.
// Env: D3_CLIENT_DIST = a built packages/kernel-client/dist (any checkout), D3_KERNEL_BIN = the chariox-kernel binary.
import { spawn } from "node:child_process"
import { once } from "node:events"
import { closeSync, mkdirSync, mkdtempSync, openSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { randomBytes } from "node:crypto"

const dist = process.env.D3_CLIENT_DIST
if (!dist) throw new Error("set D3_CLIENT_DIST to a built packages/kernel-client/dist")
const { LocalIpcClient } = await import(join(dist, "ipc.js"))
const { submitPromptRequest } = await import(join(dist, "ipc-requests.js"))
export { submitPromptRequest }

const freePort = () => new Promise(resolvePort => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => resolvePort(port)) }) })
export async function until(check, what, timeout = 60000, every = 1000) {
  const end = Date.now() + timeout
  for (;;) { const found = await check(); if (found) return found; if (Date.now() > end) throw new Error(`timeout: ${what}`); await delay(every) }
}

export async function startDrillKernel({ label, binary, stateRoot, evidence, tokenFile, model = "claude-haiku-5-5", extraEnv = {} }) {
  const devRoot = "/root/.chariox/dev/"
  mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
  stateRoot = realpathSync(stateRoot)
  if (!stateRoot.startsWith(devRoot)) throw new Error("state must be lane-owned")
  mkdirSync(evidence, { recursive: true, mode: 0o700 })
  const dir = mkdtempSync(join(stateRoot, "run-"))
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { mode: 0o700 })
  const ports = [await freePort(), await freePort(), await freePort(), await freePort()]
  mkdirSync("/var/tmp/rch", { recursive: true, mode: 0o700 })
  const socketDir = mkdtempSync("/var/tmp/rch/run-")
  const env = {
    PATH: `/root/.local/bin:${process.env.PATH}`, IS_SANDBOX: "1", SHELL: "/bin/bash", NODE_OPTIONS: "--max-old-space-size=3072",
    HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join(socketDir, "kernel.sock"), CHARIOX_DAEMON_ALIAS: label, CHARIOX_MACHINE_ALIAS: `${label}-machine`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
    CHARIOX_KERNEL_BROWSER_EXECUTABLE: "/usr/bin/google-chrome", ...extraEnv,
  }
  const log = openSync(join(evidence, "kernel.log"), "a", 0o600)
  const child = spawn(binary, [], { env, stdio: ["ignore", log, log] })
  closeSync(log)
  await once(child, "spawn")
  const t0 = Date.now(), steps = []
  const note = (name, data = {}) => { steps.push({ at: new Date().toISOString(), name, ...data }); writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 }); console.log(`${label} +${Math.round((Date.now() - t0) / 1000)}s ${name} ${JSON.stringify(data).slice(0, 240)}`) }
  note("kernel-start", { pid: child.pid, ports })
  const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  const rpc = async request => { const result = await client.send(request); if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`); return result }
  await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready", 90000)
  const passphrase = randomBytes(24).toString("hex")  // held in memory only
  await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  note("vault-unlocked")
  const value = readFileSync(tokenFile, "utf8").trim()
  await rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value, overwrite: true } })
  const profile = (await rpc({ GetProviderAccountProfile: { provider: "claude", account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
  note("claude-account-installed", { profile })
  const session = async id => (await rpc({ GetSessionState: { session_id: id } })).SessionState.session
  async function room(alias, workDir) {
    mkdirSync(workDir, { recursive: true })
    const made = (await rpc({ CreateSession: { workspace_id: workDir, worktree_id: workDir, alias, agent_defaults: { provider: "claude", account_profile: profile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
    const id = made.session.id, agent = made.agent.id
    const attachment = (await rpc({ AttachToSession: { session_id: id, client_id: `${label}-owner`, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await client.subscribeToKernelEvents(id, attachment)
    await rpc({ LaunchProviderRun: { session_id: id, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: profile, model, variant: "low", native_tui: false } })
    return { id, agent, attachment, workDir }
  }
  /** Submit a prompt and wait until the agent has no active or queued prompt; cards that appear are listed, never answered here. */
  async function ask(r, text, { timeoutMs = 600000, onCard } = {}) {
    await rpc(submitPromptRequest(r.id, r.attachment, r.agent, text, []))
    const cards = new Map()
    await delay(8000)
    await until(async () => {
      const s = await session(r.id)
      for (const c of s.active_interactions ?? []) if (!cards.has(c.id)) { cards.set(c.id, { kind: c.kind, title: c.title, at: Date.now() }); note("card", { kind: c.kind, title: String(c.title).slice(0, 80) }); await onCard?.(c) }
      const agent = (s.agents ?? []).find(a => a.id === r.agent)
      const working = (s.agent_tasks ?? []).some(t => t.agent_id === r.agent && t.state === "working")
      return !s.active_prompt && !(s.queued_prompts ?? []).length && !agent?.is_processing && !working
    }, "agent turn settles", timeoutMs, 3000)
    return [...cards.values()]
  }
  async function stop() {
    await client.close().catch(() => {})
    if (child.pid > 1 && child.exitCode === null) { child.kill("SIGTERM"); try { await until(() => child.exitCode !== null || child.signalCode !== null, "kernel exits", 30000) } catch { child.kill("SIGKILL") } }
    rmSync(socketDir, { recursive: true, force: true })
    note("kernel-exit", { code: child.exitCode, signal: child.signalCode })
  }
  return { dir, env, rpc, note, session, room, ask, stop, client, profile, child, ports }
}

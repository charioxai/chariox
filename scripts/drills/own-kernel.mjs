// A lane-owned kernel for drills that need one real kernel and one real Claude agent: its own CHARIOX_HOME, ports and
// state under /root/.chariox/dev/, a linked product Claude profile, and a notes file next to the evidence.
import { spawn } from "node:child_process"
import { once } from "node:events"
import { closeSync, cpSync, mkdirSync, mkdtempSync, openSync, realpathSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"

export function drillFlags(argv) {
  const flags = new Map()
  for (let i = 2; i < argv.length; i++) flags.set(argv[i], argv[++i])
  const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
  return { flags, need }
}

export async function until(check, what, timeout = 30000) {
  const end = Date.now() + timeout
  for (;;) {
    const found = await check()
    if (found) return found
    if (Date.now() > end) throw new Error(`timeout: ${what}`)
    await delay(1000)
  }
}

async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const { port } = server.address()
  await new Promise(r => server.close(r))
  return port
}

/** Start with fresh private state and keep only the evidence when the drill stops. */
export async function startDrillKernel({ label, binary, stateRoot, evidence, profilePath, enableFaults = false, extraEnv = {}, readyTimeoutMs = 60000 }) {
  const devRoot = "/root/.chariox/dev/"
  stateRoot = resolve(stateRoot)
  if (!stateRoot.startsWith(devRoot)) throw new Error("state must be lane-owned")
  mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
  stateRoot = realpathSync(stateRoot)
  if (!stateRoot.startsWith(devRoot)) throw new Error("state must be lane-owned")
  mkdirSync(evidence, { recursive: true, mode: 0o700 })
  const dir = mkdtempSync(join(stateRoot, "run-"))
  const steps = [], t0 = Date.now()
  const note = (name, data = {}) => {
    steps.push({ at: new Date().toISOString(), name, ...data })
    writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 })
    console.log(`${label} +${Math.round((Date.now() - t0) / 1000)}s ${name} ${JSON.stringify(data).slice(0, 300)}`)
  }
  let client, child, socketDir, stopping
  function stop() {
    stopping ??= stopAndRemove()
    return stopping
  }
  async function stopAndRemove() {
    await client?.close().catch(() => {})
    if (child?.pid > 1) {
      const exited = () => child.exitCode !== null || child.signalCode !== null
      if (!exited()) {
        child.kill("SIGTERM")
        try {
          await until(exited, "kernel exits", 30000)
        } catch {
          child.kill("SIGKILL")
          await until(exited, "kernel exits after forced stop", 5000)
        }
      }
    }
    if (socketDir) rmSync(socketDir, { recursive: true, force: true })
    // The kernel's own log is the evidence of why a wake was not delivered: keep it next to the notes.
    try { cpSync(join(dir, "logs"), join(evidence, "kernel-logs"), { recursive: true }) } catch {}
    rmSync(dir, { recursive: true, force: true })
    note("kernel-exit", { code: child?.exitCode, signal: child?.signalCode })
  }

  try {
    return await initialize()
  } catch (error) {
    try {
      await stop()
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], `Drill setup and cleanup failed for ${dir}`)
    }
    throw error
  }
  async function initialize() {
    for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) {
      mkdirSync(join(dir, sub), { mode: 0o700 })
    }
    const ports = [await freePort(), await freePort(), await freePort(), await freePort()]
    // Keep the private Unix socket path short enough even for deeply nested state roots.
    mkdirSync("/var/tmp/rch", { recursive: true, mode: 0o700 })
    socketDir = mkdtempSync(join("/var/tmp/rch", "run-"))
    const env = {
      PATH: process.env.PATH, IS_SANDBOX: "1", SHELL: "/bin/bash", NODE_OPTIONS: "--max-old-space-size=3072",
      HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
      CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
      CHARIOX_DAEMON_SOCKET: join(socketDir, "kernel.sock"), CHARIOX_DAEMON_ALIAS: label, CHARIOX_MACHINE_ALIAS: `${label}-machine`,
      CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
    }
    Object.assign(env, extraEnv)
    if (enableFaults) {
      env.CHARIOX_AGENT_FAULT_DIR = join(dir, "faults")
      mkdirSync(env.CHARIOX_AGENT_FAULT_DIR, { mode: 0o700 })
    }
    const log = openSync(join(evidence, "kernel.log"), "a", 0o600)
    try {
      child = spawn(binary, [], { env, stdio: ["ignore", log, log] })
      await once(child, "spawn")
    } finally {
      closeSync(log)
    }
    note("kernel-start", { pid: child.pid })
    client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
    const rpc = async request => {
      const result = await client.send(request)
      if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
      return result
    }
    await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready", readyTimeoutMs)
    const profile = profilePath
      ? (await rpc({ LinkProviderAccountProfile: { provider: "claude", label, path: profilePath } })).ProviderAccountProfile.profile.profile_id
      : undefined
    const session = async id => (await rpc({ GetSessionState: { session_id: id } })).SessionState.session
    /** A Room with one launched Claude agent and an attached owner client. */
    async function room(alias, model) {
      if (!profile) throw new Error("A Claude profile is required to launch a drill agent")
      const work = join(dir, "work", alias)
      mkdirSync(work, { recursive: true })
      const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias, agent_defaults: { provider: "claude", account_profile: profile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
      const R = made.session.id, agent = made.agent.id
      const attached = (await rpc({ AttachToSession: { session_id: R, client_id: `${label}-owner`, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
      await client.subscribeToKernelEvents(R, attached)
      await rpc({ LaunchProviderRun: { session_id: R, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: profile, model, variant: "low", native_tui: false } })
      return { R, agent, attached, work }
    }
    return { dir, env, note, rpc, client, session, room, stop, profile, pid: child.pid }
  }
}

/** A request that waits for an interaction (a sudo passkey card) stays pending while its card is found and answered, then is awaited. */
export async function answerWhilePending({ request, findCard, answer }) {
  request.catch(() => {})
  await answer(await findCard())
  return request
}

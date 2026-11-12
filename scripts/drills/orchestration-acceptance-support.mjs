// Private real kernels, encrypted relay, provider account enrollment and shipped TUI.
import assert from "node:assert/strict"
import { spawn, execFileSync } from "node:child_process"
import { createHash, createHmac, randomBytes, randomUUID } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer, createConnection } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { startOwnerCloud } from "./local-owner-cloud.test-support.mjs"

export async function until(check, label, timeout = 120000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(500) }
  throw new Error(`timeout: ${label}`)
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
export async function createDrill(flags) {
  const required = key => { assert(flags.get(key), `${key} is required`); return resolve(flags.get(key)) }
  const stateRoot = required("--state-root"), evidence = required("--evidence-dir")
  assert(stateRoot.startsWith("/root/.chariox/dev/"))
  mkdirSync(stateRoot, { recursive: true, mode: 0o700 }); mkdirSync(evidence, { recursive: true })
  const root = mkdtempSync(join(stateRoot, "ao-")), processes = [], clients = [], steps = []
  const source = execFileSync("git", ["rev-parse", "HEAD"]).toString().trim()
  const record = (name, data = {}) => {
    steps.push({ at: new Date().toISOString(), name, ...data })
    writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2))
    console.log(name, JSON.stringify(data).slice(0, 300))
  }
  const save = (name, value) => writeFileSync(join(evidence, name), JSON.stringify(value, null, 2))
  const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash" }
  function start(binary, args, extra, label, cwd) {
    const fd = openSync(join(evidence, `${label}.log`), "a", 0o600)
    const child = spawn(binary, args, { env: { ...env, ...extra }, cwd, stdio: ["ignore", fd, fd] })
    assert(Number.isSafeInteger(child.pid) && child.pid > 1)
    processes.push(child); record("started", { label, pid: child.pid }); return child
  }
  let cloud, tui
  const cleanup = async () => {
    const owned = new Set(processes.map(p => p.pid))
    const parents = execFileSync("ps", ["-eo", "pid=,ppid="]).toString().trim().split("\n").map(line => line.trim().split(/\s+/).map(Number))
    for (let n = 0; n < parents.length; n++) for (const [pid, parent] of parents) if (owned.has(parent)) owned.add(pid)
    const alive = pid => { try { process.kill(pid, 0); return true } catch { return false } }
    const signal = (pid, value) => { try { process.kill(pid, value) } catch (e) { if (e.code !== "ESRCH") throw e } }
    for (const name of ["a", "b"]) {
      const db = join(root, name, "chariox/history/operational.db")
      if (existsSync(db)) save(`public-history-${name}.json`, JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", db, "SELECT sequence,session_id,agent_id,kind,text,document_json FROM public_history ORDER BY sequence"]).toString() || "[]"))
    }
    if (tui) { await tui.send("exit").catch(() => {}); tui.socket.destroy() }
    for (const c of clients) await c.close().catch(() => {})
    for (const p of processes.toReversed()) {
      if (p.exitCode !== null || p.signalCode !== null) continue
      p.kill("SIGTERM")
      try { await until(() => p.exitCode !== null || p.signalCode !== null, `exit ${p.pid}`, 15000) }
      catch { p.kill("SIGKILL"); await until(() => p.exitCode !== null || p.signalCode !== null, `kill ${p.pid}`, 5000) }
    }
    for (const pid of owned) if (Number.isSafeInteger(pid) && pid > 1 && alive(pid)) signal(pid, "SIGTERM")
    await delay(500)
    const remaining = [...owned].filter(alive)
    for (const pid of remaining) if (Number.isSafeInteger(pid) && pid > 1) signal(pid, "SIGKILL")
    cloud?.closeAllConnections(); cloud?.close()
    rmSync(root, { recursive: true, force: true })
    record("cleanup", { stateRemoved: !existsSync(root), liveChildren: processes.filter(p => p.exitCode === null && p.signalCode === null).map(p => p.pid) })
  }
  try {
    const secret = randomBytes(32).toString("base64url"), realm = `ao-${randomUUID()}`
    const token = (subject, kind, actions, thumbprint, targets) => {
      const h = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
      const b = Buffer.from(JSON.stringify({ iss: realm, sub: subject, subject_kind: kind, realm_id: realm, allowed_actions: actions, allowed_targets: targets ?? null, ...(thumbprint ? { public_key_thumbprint: thumbprint } : {}), iat: Math.floor(Date.now()/1000)-1, exp: Math.floor(Date.now()/1000)+7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
      return `${h}.${b}.${createHmac("sha256", secret).update(`${h}.${b}`).digest("base64url")}`
    }
    const relayPort = await freePort()
    start(required("--relay-binary"), [], { CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: realm, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
    const cloudEnv = await startOwnerCloud({ token, note: record, realm, relayPort, onServer: s => { cloud = s } })
    async function kernel(name) {
      const dir = join(root, name)
      for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
      writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
      const ports = await Promise.all([freePort(), freePort(), freePort(), freePort()])
      const localEnv = { ...env, ...cloudEnv, HOME: join(dir,"home"), TMPDIR: join(dir,"tmp"), XDG_CONFIG_HOME: join(dir,"config"), XDG_CACHE_HOME: join(dir,"cache"), XDG_STATE_HOME: join(dir,"state"), CHARIOX_HOME: join(dir,"chariox"), CHARIOX_LOG_DIR: join(dir,"logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir,"history"), CHARIOX_DAEMON_SOCKET: `/var/tmp/aod2/ao-${process.pid}-${name}.sock`, CHARIOX_DAEMON_ALIAS: `ao-${name}`, CHARIOX_MACHINE_ALIAS: `ao-machine-${name}`, CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
      mkdirSync("/var/tmp/aod2", { recursive: true })
      start(required("--kernel-binary"), [], localEnv, name)
      const url = `ws://127.0.0.1:${ports[0]}/kernel`, client = new LocalIpcClient(url, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 1000 })
      clients.push(client)
      const rpc = async request => {
        const result = await client.send(request)
        if (result.Error) throw new Error(`${Object.keys(request)[0]}: ${result.Error.code}: ${result.Error.message}`)
        return result
      }
      let status
      await until(async () => { try { status = (await rpc({ RelayStatus: null })).RelayStatus.status; return status.connected } catch { return false } }, `${name} connected`)
      const passphrase = randomBytes(24).toString("hex")
      await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
      await rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
      record("kernel-ready", { name, kernel: status.daemon_id })
      return { dir, env: localEnv, url, client, rpc, status }
    }
    const a = await kernel("a"), b = await kernel("b")
    const provider = flags.get("--provider") ?? "codex", model = provider === "codex" ? "gpt-6-luna" : "claude-haiku-5-5"
    let profile, profileB
    if (provider === "codex") {
      const link = async k => {
        const p = (await k.rpc({ LinkProviderAccountProfile: { provider, label: "ao-codex", path: required("--codex-profile-path") } })).ProviderAccountProfile.profile
        await k.rpc({ RefreshProviderAccountProfile: { provider, account_profile: p.profile_id } })
        return p.profile_id
      }
      profile = await link(a); profileB = await link(b)
    } else {
      const value = readFileSync(process.env.CLAUDE_TOKEN_FILE ?? "/root/.claude-lanes/oauth-token.chariox", "utf8").trim()
      await a.rpc({ SetProviderAccountCredential: { provider, account_profile: "default", value, overwrite: true } })
      profile = (await a.rpc({ GetProviderAccountProfile: { provider, account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
      const base = JSON.parse(cloudEnv.CHARIOX_CLOUD_RELAY_CONFIG_JSON).api_url
      let target
      await until(async () => {
        const directory = await (await fetch(base + "/v1/owner-managed-context-tickets/peers")).json()
        target = directory.peers.find(p => p.peer.kernelId === b.status.daemon_id)?.peer
        return !!target
      }, "target enrollment")
      const result = await a.rpc({ CopyProviderAccountToKernel: { provider, account_profile: profile, operation_id: `ao-copy-${randomUUID()}`, target_kernel_id: target.kernelId, target_key_thumbprint: target.keyThumbprint, replace: false } })
      profileB = result.ProviderAccountCopied.receipt.account_profile
      assert(profileB)
      save("account-copy.json", result); record("account-copied", { profile, profileB })
    }
    const work = join(a.dir, "work")
    execFileSync("git", ["init", "-q", "-b", "main", work])
    writeFileSync(join(work, "README.md"), "AO-PE-03 / AO-IN-02 real provider acceptance\n")
    execFileSync("git", ["-C", work, "add", "README.md"])
    execFileSync("git", ["-C", work, "-c", "user.name=drill", "-c", "user.email=drill@example.invalid", "commit", "-qm", "start"])
    const makeRoom = async (k, alias, account) => {
      const made = (await k.rpc({ CreateSession: { workspace_id: join(k.dir,"work"), worktree_id: join(k.dir,"work"), alias, agent_defaults: { provider, account_profile: account, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
      const attachment = (await k.rpc({ AttachToSession: { session_id: made.session.id, client_id: "ao-driver", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
      await k.client.subscribeToKernelEvents(made.session.id, attachment)
      await k.rpc({ LaunchProviderRun: { session_id: made.session.id, agent_id: made.agent.id, adapter_key: provider, provider, account_profile: account, model, variant: "low", native_tui: false } })
      return { id: made.session.id, agent: made.agent.id, attachment }
    }
    const room = await makeRoom(a, "ao-parent", profile), other = await makeRoom(b, "ao-other", profileB)
    const state = async () => (await a.rpc({ GetSessionState: { session_id: room.id } })).SessionState.session
    const submit = async text => {
      if (tui) return tui.send("submit_prompt", { prompt: text })
      return a.rpc(submitPromptRequest(room.id, room.attachment, room.agent, text, []))
    }
    const db = join(a.dir, "chariox/state.db")
    const query = sql => JSON.parse(execFileSync("sqlite3", ["-readonly", "-json", db, sql]).toString() || "[]")
    if (flags.get("--tui") === "yes") {
      const sock = `/var/tmp/aod2/tui-${process.pid}.sock`, cli = resolve("apps/cli/dist/index.js")
      const quote = s => "'" + s.replaceAll("'", "'\\''") + "'"
      start("script", ["-q", "-c", ["bun", cli, "--kernel-url", a.url, "--session", room.id, "--automation-socket", sock, "--client-id", "ao-real-tui"].map(quote).join(" "), "/dev/null"], { ...a.env, TERM: "xterm-256color", COLUMNS: "120", LINES: "40", CHARIOX_TEST_TUI: "1" }, "tui", work)
      await until(() => existsSync(sock), "real TUI socket")
      const socket = createConnection(sock); socket.setEncoding("utf8")
      let id = 0, buffer = ""; const pending = new Map()
      socket.on("data", chunk => {
        buffer += chunk
        while (buffer.includes("\n")) {
          const i = buffer.indexOf("\n"), line = buffer.slice(0,i); buffer = buffer.slice(i+1)
          const r = JSON.parse(line), p = pending.get(r.id); pending.delete(r.id)
          if (p) clearTimeout(p.timeout)
          if (r.ok) p?.resolve(r.data); else p?.reject(new Error(r.error))
        }
      })
      const lost = error => { for (const p of pending.values()) { clearTimeout(p.timeout); p.reject(error) }; pending.clear() }
      socket.on("error", lost); socket.on("close", () => lost(new Error("TUI connection closed")))
      tui = { socket, send: (action, fields = {}) => new Promise((resolve, reject) => {
        const n = ++id, timeout = setTimeout(() => { pending.delete(n); reject(new Error(`TUI ${action} timed out`)) }, 30000)
        pending.set(n, { resolve, reject, timeout }); socket.write(JSON.stringify({ id: n, action, ...fields })+"\n")
      }) }
      await tui.send("ping"); save("tui-initial.json", await tui.send("snapshot")); record("tui-ready")
    }
    save("provenance.json", { source, provider, model, profile, profileB, room, other, kernelA: a.status.daemon_id, kernelB: b.status.daemon_id, kernelSha256: createHash("sha256").update(readFileSync(required("--kernel-binary"))).digest("hex"), relaySha256: createHash("sha256").update(readFileSync(required("--relay-binary"))).digest("hex"), controlPlane: "loopback owner enrollment fixture; real kernels, encrypted relay, provider CLI and account copy" })
    return { a, b, room, other, provider, model, profile, profileB, root, work, evidence, record, save, state, submit, query, cleanup, tui }
  } catch (e) { await cleanup(); throw e }
}

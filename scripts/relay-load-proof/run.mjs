// MP-08 / MP-09 / MP-10 / MP-11: relay load proof. A throwaway local relay and two dev kernels; the client
// kernel reaches the relay through a shaped link while it downloads a large file from the serving kernel
// with `chariox files get` through the kernel gateway. While the transfer runs, small probes measure what
// else on that link stays responsive. Never the live relay or a live kernel: every process and home is
// created here and removed at the end. Credentials are generated for this run and never written out.
//   node run.mjs --kernel-binary K --relay-binary R --label base --profile 8m --variant single,three,stalled
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHmac, createHash, randomBytes, randomFillSync, randomUUID } from "node:crypto"
import { closeSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, readSync, rmSync, statSync, unlinkSync, writeFileSync, writeSync } from "node:fs"
import { createServer } from "node:net"
import { dirname, join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const repo = resolve(here, "../..")
const { LocalIpcClient } = await import(join(repo, "packages/kernel-client/dist/ipc.js"))

const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const need = key => { const v = flags.get(key); assert.equal(typeof v, "string", `${key} is required`); return v }
const kernelBinary = resolve(need("--kernel-binary")), relayBinary = resolve(need("--relay-binary"))
const cliDist = resolve(flags.get("--cli-dist") ?? join(repo, "apps/cli/dist/index.js"))
const label = flags.get("--label") ?? "run"
const evidenceRoot = resolve(flags.get("--evidence-dir") ?? "/root/.codex/evidence/relayproof/r1")
const stateRoot = resolve(flags.get("--state-root") ?? "/root/.chariox/dev/relayproof/runs")
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state must stay under /root/.chariox/dev")
// "8m": 8 Mbit/s down, 2 up. "1m": 1 Mbit/s down, 1 up. RTT is 60 ms in all.
// "1m-bloat" is "1m" behind a 1 MiB queue (about 8 s of delay when full), like a modem with a deep buffer.
const PROFILES = { "8m": { down: 8000, up: 2000, queueKiB: 256 }, "1m": { down: 1000, up: 1000, queueKiB: 256 }, "1m-bloat": { down: 1000, up: 1000, queueKiB: 1024 },
  // The incident link: a congested uplink of about 600 kbit/s (the Mac pulled 76 KB/s) with a deep buffer.
  "600k": { down: 600, up: 600, queueKiB: 1024 },
  // The same links with a bottleneck buffer near the bandwidth-delay product: the bulk that does not fit waits in the
  // sender's socket buffer (visible as Send-Q on the relay), where only the sender can order it.
  "8m-sym": { down: 8000, up: 8000, queueKiB: 256 }, // client uplink as fast as the downlink, for `files put`
  "8m-sym-bdp": { down: 8000, up: 8000, queueKiB: 64 }, "8m-sym-deep": { down: 8000, up: 8000, queueKiB: 1024 },
  "8m-bdp": { down: 8000, up: 2000, queueKiB: 64 }, "1m-bdp": { down: 1000, up: 1000, queueKiB: 16 } }
const profileName = flags.get("--profile") ?? "8m"
const profile = PROFILES[profileName]
assert(profile, `--profile is one of ${Object.keys(PROFILES)}`)
const rttMs = Number(flags.get("--rtt-ms") ?? 60), jitterMs = Number(flags.get("--jitter-ms") ?? 6)
const fullSizeBytes = Math.round(Number(flags.get("--size-mb") ?? 100) * 1024 * 1024)
const direction = flags.get("--direction") ?? "get" // "get": relay -> client link carries the file; "put": client -> relay link
assert(["get", "put"].includes(direction), "--direction is get or put")
const variants = (flags.get("--variant") ?? "single,three,stalled").split(",")
const stallSeconds = Number(flags.get("--stall-seconds") ?? 20)
// The frozen-reader variant moves less data (the transfer has to resume after the thaw) and queues a burst of replies for the frozen client.
const stalledSizeBytes = Math.round(Number(flags.get("--stalled-size-mb") ?? 30) * 1024 * 1024)
const stallBurst = Number(flags.get("--stall-burst") ?? 3000)
const PROBE_INTERVAL_MS = 250, PROBE_TIMEOUT_MS = 15000
for (const v of variants) assert(["single", "three", "stalled"].includes(v), `unknown variant ${v}`)

const guardedKill = (pid, signal) => {
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error(`refusing to signal pid ${pid}`)
  process.kill(pid, signal)
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(check, what, timeout = 40000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(100) }
  throw new Error(`timeout: ${what}`)
}
const percentile = (sorted, p) => sorted.length ? sorted[Math.min(sorted.length - 1, Math.ceil(p * sorted.length) - 1)] : null
const summarize = samples => {
  const done = samples.filter(s => s.ok).map(s => s.ms).sort((a, b) => a - b)
  return { n: samples.length, failed: samples.length - done.length, p50: percentile(done, 0.5), p95: percentile(done, 0.95), max: done.at(-1) ?? null }
}
const cpuSeconds = pid => { try { const f = readFileSync(`/proc/${pid}/stat`, "utf8").slice(readFileSync(`/proc/${pid}/stat`, "utf8").lastIndexOf(")") + 2).split(" "); return (Number(f[11]) + Number(f[12])) / 100 } catch { return null } }
const rssKib = pid => { try { return Number(/VmRSS:\s+(\d+)/.exec(readFileSync(`/proc/${pid}/status`, "utf8"))[1]) } catch { return null } }

async function runVariant(variant) {
  const evidence = join(evidenceRoot, `${label}-${profileName}-${variant}`)
  mkdirSync(evidence, { recursive: true })
  const root = mkdtempSync(join(stateRoot, "proof-"))
  const children = [], clients = []
  const secret = randomBytes(32).toString("base64url"), issuer = `relay-proof-${randomUUID()}`
  const token = (subject, kind, actions) => {
    const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
    const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer, allowed_actions: actions,
      allowed_targets: null, iat: Math.floor(Date.now() / 1000) - 1, exp: Math.floor(Date.now() / 1000) + 7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
    return `${head}.${body}.${createHmac("sha256", secret).update(`${head}.${body}`).digest("base64url")}`
  }
  const baseEnv = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash",
    // Opt-in per-stage packet timing (MP-10-PACKET-TIMING lines in the kernel and relay logs), joined by ciphertext digest.
    ...(flags.get("--packet-timing") === "1" ? { CHARIOX_BROWSER_DISPLAY_TIMING: "1" } : {}) }
  const start = (program, args, extraEnv, name) => {
    const log = openSync(join(evidence, `${name}.log`), "a", 0o600)
    const child = spawn(program, args, { env: { ...baseEnv, ...extraEnv }, stdio: ["ignore", log, log], cwd: repo })
    assert(Number.isSafeInteger(child.pid) && child.pid > 1)
    child.name = name
    children.push(child)
    return child
  }
  async function stop(child) {
    if (child.exitCode !== null || child.signalCode !== null) return
    try { guardedKill(child.pid, "SIGCONT"); guardedKill(child.pid, "SIGTERM") } catch {}
    try { await until(() => child.exitCode !== null || child.signalCode !== null, `${child.name} exits`, 15000) } catch {
      guardedKill(child.pid, "SIGKILL")
      await until(() => child.exitCode !== null || child.signalCode !== null, `${child.name} exits after kill`, 5000)
    }
  }
  async function role(name, relayUrlFor) {
    const dir = join(root, name), ports = []
    for (let i = 0; i < 4; i++) ports.push(await freePort())
    for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
    writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
    const env = { ...baseEnv, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"),
      XDG_STATE_HOME: join(dir, "state"), CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
      CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"), CHARIOX_DAEMON_ALIAS: `proof-${name}`, CHARIOX_MACHINE_ALIAS: `proof-${name}`,
      CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
    const r = { name, dir, env, passkey: randomBytes(24).toString("hex"), url: `ws://127.0.0.1:${ports[0]}/kernel` }
    r.child = start(kernelBinary, [], env, name)
    const open = () => {
      const client = new LocalIpcClient(r.url, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
      clients.push(client)
      return client
    }
    r.open = open
    r.client = open()
    r.rpc = async request => {
      const result = await r.client.send(request)
      if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code}`)
      return result
    }
    await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${name} ready`)
    r.status = (await r.rpc({ RelayStatus: null })).RelayStatus.status
    return r
  }
  async function unlock(r) {
    const made = (await r.rpc({ CreateSession: { workspace_id: join(r.dir, "work"), worktree_id: join(r.dir, "work"), alias: "proof-owner" } })).SessionCreated
    const att = (await r.rpc({ AttachToSession: { session_id: made.session.id, client_id: "relay-proof", capability_level: "FullTerminal" } })).SessionAttached
    await r.client.subscribeToKernelEvents(made.session.id, att.attachment.id)
    const pending = r.rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } })
    pending.catch(() => {})
    let interaction
    await until(async () => {
      const st = (await r.rpc({ GetSessionState: { session_id: made.session.id } })).SessionState.session
      interaction = st.active_interactions?.find(x => x.id.startsWith("vault-unlock-"))
      return interaction
    }, "vault prompt")
    await r.rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: interaction.id, choice_id: "passphrase", custom_reply: r.passkey } })
    await pending
    r.client.onKernelEvent(async event => {
      if (event.event !== "passkey_prompts_changed") return
      for (const prompt of event.prompts || []) if (prompt.kind === "access_grant") {
        await r.rpc({ RespondToInteraction: { session_id: prompt.session_id || "kernel-access", interaction_id: prompt.interaction_id, choice_id: "approve", passkey: r.passkey } }).catch(() => {})
      }
    })
  }
  const cli = (r, args) => new Promise((done, fail) => {
    const started = Date.now()
    const child = spawn("bun", [cliDist, ...args], { env: { ...r.env, CHARIOX_CLI_ORIGINAL_CWD: repo }, cwd: repo, stdio: ["ignore", "pipe", "pipe"] })
    children.push(child)
    child.name = "cli"
    let out = "", err = ""
    child.stdout.on("data", b => out += b)
    child.stderr.on("data", b => err += b)
    child.on("error", fail)
    child.on("close", code => { let value; try { value = JSON.parse(out) } catch {} ; done({ code, value, err: err.slice(-400), ms: Date.now() - started, pid: child.pid }) })
  })

  const sizeBytes = variant === "stalled" ? Math.min(stalledSizeBytes, fullSizeBytes) : fullSizeBytes
  const result = { label, profile: profileName, link: { ...profile, rttMs, jitterMs }, variant, direction, sizeBytes, startedAt: new Date().toISOString(), errors: [], notes: [] }
  const filesToRemove = []
  try {
    const relayPort = await freePort()
    mkdirSync(join(root, "relay"), { mode: 0o700 })
    const relay = start(relayBinary, [], { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort),
      CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
    await until(async () => { try { const s = await import("node:net").then(n => new Promise(ok => { const c = n.connect(relayPort, "127.0.0.1"); c.on("connect", () => { c.destroy(); ok(true) }); c.on("error", () => ok(false)) })); return s } catch { return false } }, "relay listening")
    // The client kernel reaches the relay through the shaped link; the serving kernel is next to the relay.
    const proxyLog = openSync(join(evidence, "proxy.log"), "a", 0o600)
    const proxy = spawn("node", [join(here, "throttle-proxy.mjs"), "--target", `127.0.0.1:${relayPort}`, "--down-kbps", String(profile.down), "--up-kbps", String(profile.up), "--queue-bytes", String(profile.queueKiB * 1024),
      "--delay-ms", String(rttMs / 2), "--jitter-ms", String(jitterMs / 2)], { stdio: ["ignore", "pipe", proxyLog] })
    children.push(proxy); proxy.name = "proxy"
    const proxyPort = await new Promise((ok, fail) => { proxy.stdout.once("data", b => ok(JSON.parse(String(b)).listening)); proxy.on("error", fail) })
    const proxyStats = []
    let proxyBuffer = ""
    proxy.stdout.on("data", b => { proxyBuffer += b; let i; while ((i = proxyBuffer.indexOf("\n")) >= 0) { const line = proxyBuffer.slice(0, i); proxyBuffer = proxyBuffer.slice(i + 1); try { const m = JSON.parse(line); if (m.bytes) proxyStats.push(m.bytes) } catch {} } })
    const proxyBytes = async () => { const n = proxyStats.length; guardedKill(proxy.pid, "SIGUSR1"); await until(() => proxyStats.length > n, "proxy counters", 5000); return proxyStats.at(-1) }
    const worker = await role("worker"), home = await role("home")
    const allActions = ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event", "client.connect", "client.metadata.read"]
    await worker.rpc({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${relayPort}`, relay_token: token(worker.status.daemon_id, "kernel", allActions) } })
    // The pairing link names the relay it was made on, so the pair is made on the direct address and the client kernel moves behind the shaped link afterwards.
    await home.rpc({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${relayPort}`, relay_token: token(home.status.daemon_id, "kernel", allActions) } })
    for (const r of [worker, home]) await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, `${r.name} registered`)
    const pairing = (await worker.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 600000 } })).TerminalPairingLinkCreated.pairing
    await home.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("proof-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
    await home.rpc({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${proxyPort}`, relay_token: token(home.status.daemon_id, "kernel", allActions) } })
    await until(async () => (await home.rpc({ RelayStatus: null })).RelayStatus.status.connected, "home behind the shaped link")
    await unlock(home)
    let r = await cli(home, ["access", "request", "--until-exit", "--holder-pid", String(process.pid)])
    assert.equal(r.code, 0, `access request: ${r.err}`)
    const target = worker.status.daemon_id

    // The probes: a small request to the serving kernel through the gateway, the kernel list, and the serving kernel's own link.
    const smallClient = new LocalIpcClient(`ws+unix://${join(home.dir, "daemon.sock")}`, { kernel: target, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 3600 * 1000 })
    const listClient = new LocalIpcClient(`ws+unix://${join(home.dir, "daemon.sock")}`, { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 3600 * 1000 })
    clients.push(smallClient, listClient)
    const timed = async (client, body, bucket) => {
      const started = performance.now()
      const timer = delay(PROBE_TIMEOUT_MS).then(() => { throw new Error("probe timeout") })
      try { const response = await Promise.race([client.send(body), timer]); if (response?.Error) throw new Error(response.Error.code); bucket.push({ at: Date.now(), ms: performance.now() - started, ok: true }) }
      catch (error) { bucket.push({ at: Date.now(), ms: performance.now() - started, ok: false, error: String(error.message).slice(0, 80) }) }
    }
    const probes = { small: [], list: [], idleSmall: [], idleList: [] }
    const small = { ListSessions: null }, listBody = { ListGatewayKernels: {} }
    await timed(smallClient, small, []); await timed(listClient, listBody, []) // warm the connections
    const idleFrom = await proxyBytes()
    for (let i = 0; i < 20; i++) { await timed(smallClient, small, probes.idleSmall); await timed(listClient, listBody, probes.idleList); await delay(PROBE_INTERVAL_MS) }

    const idleTo = await proxyBytes()
    const probeWireDown = (idleTo[direction === "put" ? "up" : "down"] - idleFrom[direction === "put" ? "up" : "down"]) / 20 // bytes the relay sends per probe pair (small request + kernel list)
    // The data: a file of random bytes on the serving kernel.
    const bigPath = join(direction === "put" ? home.dir : worker.dir, "work", "big.bin")
    filesToRemove.push(bigPath)
    const fd = openSync(bigPath, "w", 0o600), block = Buffer.alloc(1024 * 1024)
    for (let written = 0; written < sizeBytes; written += block.length) { randomFillSync(block); writeSync(fd, block, 0, Math.min(block.length, sizeBytes - written)) }
    closeSync(fd)
    const transferCount = variant === "three" ? 3 : 1
    const destinations = Array.from({ length: transferCount }, (_, i) => join(direction === "put" ? worker.dir : home.dir, "work", `got-${i}.bin`))
    filesToRemove.push(...destinations, ...destinations.map(d => `${d}.chariox-partial`))

    result.relayRssKib = { before: rssKib(relay.pid), max: rssKib(relay.pid) }
    const cpuBefore = { relay: cpuSeconds(relay.pid), worker: cpuSeconds(worker.child.pid), home: cpuSeconds(home.child.pid) }
    result.proxyBytesBefore = null
    const transfers = destinations.map(dest => ({ dest, done: false }))
    const wireFrom = await proxyBytes()
    const startedAt = Date.now()
    const relayLogFrom = statSync(join(evidence, "relay.log")).size
    let probing = true
    const sampleRss = setInterval(() => { result.relayRssKib.max = Math.max(result.relayRssKib.max, rssKib(relay.pid) ?? 0) }, 500)
    const probeLoop = (async () => {
      let tick = 0
      while (probing) {
        timed(smallClient, small, probes.small)
        timed(listClient, listBody, probes.list)
        // Presence of the serving kernel as its own local IPC sees it: connected stays true through the run.
        if (tick++ % 8 === 0) worker.rpc({ RelayStatus: null }).then(s => (probes.workerLink ??= []).push({ at: Date.now(), connected: s.RelayStatus.status.connected }), () => (probes.workerLink ??= []).push({ at: Date.now(), connected: null }))
        await delay(PROBE_INTERVAL_MS)
      }
    })()
    const downloads = transfers.map(t => cli(home, ["files", direction, "--kernel", target, bigPath, t.dest, "--overwrite", ...(variant === "stalled" ? ["--timeout", "120"] : [])]).then(res => {
      t.done = true; t.code = res.code; t.elapsedMs = res.ms; t.error = res.code === 0 ? null : res.err; t.finishedAt = Date.now(); t.cliPid = res.pid
      try { t.bytes = statSync(t.dest).size } catch { t.bytes = 0 }
    }))
    if (variant === "stalled") {
      await delay(10000)
      result.notes.push(`SIGSTOP home kernel pid ${home.child.pid} at +${Date.now() - startedAt} ms for ${stallSeconds} s`)
      // Replies pile up for the client: a burst of small requests whose answers come back while it is frozen.
      const burst = Array.from({ length: stallBurst }, () => smallClient.send(small).then(() => true, () => false))
      await delay(1500)
      guardedKill(home.child.pid, "SIGSTOP")
      const stalledFrom = Date.now()
      while (Date.now() - stalledFrom < stallSeconds * 1000) {
        // The serving kernel is probed on its own socket while its reader is frozen.
        const sample = { at: Date.now() }
        const t0 = performance.now()
        try { const s = await worker.rpc({ RelayStatus: null }); sample.connected = s.RelayStatus.status.connected; sample.ms = performance.now() - t0 } catch (e) { sample.connected = null; sample.error = String(e.message).slice(0, 60) }
        ;(probes.workerDuringStall ??= []).push(sample)
        await delay(1000)
      }
      result.relayRssKib.duringStall = rssKib(relay.pid)
      guardedKill(home.child.pid, "SIGCONT")
      result.notes.push(`SIGCONT at +${Date.now() - startedAt} ms`)
      const answered = await Promise.race([Promise.all(burst), delay(30000).then(() => null)])
      result.burst = { sent: stallBurst, answered: answered ? answered.filter(Boolean).length : null }
    }
    await Promise.all(downloads)
    probing = false
    await probeLoop
    clearInterval(sampleRss)
    const windowMs = Date.now() - startedAt
    const wireTo = await proxyBytes()
    result.cpuSeconds = Object.fromEntries(Object.entries({ relay: relay.pid, worker: worker.child.pid, home: home.child.pid }).map(([k, pid]) => [k, +((cpuSeconds(pid) ?? 0) - cpuBefore[k]).toFixed(1)]))
    await delay(2000)
    result.relayRssKib.after = rssKib(relay.pid)
    result.transfers = transfers.map(t => ({ code: t.code, elapsedMs: t.elapsedMs, bytes: t.bytes, error: t.error }))
    for (const t of transfers) if (t.code === 0) {
      const want = createHash("sha256"); const got = createHash("sha256")
      for (const [hash, path] of [[want, bigPath], [got, t.dest]]) { const f = openSync(path, "r"); const b = Buffer.alloc(4 << 20); let n; while ((n = readSync(f, b, 0, b.length, null)) > 0) hash.update(b.subarray(0, n)); closeSync(f) }
      result.transfers[transfers.indexOf(t)].hashOk = want.digest("hex") === got.digest("hex")
    }
    const bytesPerSecondLink = (direction === "put" ? profile.up : profile.down) * 1000 / 8
    result.windowMs = windowMs
    result.idealSeconds = sizeBytes * transferCount / bytesPerSecondLink
    result.payloadRate = result.transfers.map(t => t.code === 0 ? sizeBytes / (t.elapsedMs / 1000) / bytesPerSecondLink : 0)
    result.probes = { idleSmall: summarize(probes.idleSmall), idleList: summarize(probes.idleList), small: summarize(probes.small), list: summarize(probes.list),
      smallErrors: [...new Set(probes.small.filter(p => !p.ok).map(p => p.error))], listErrors: [...new Set(probes.list.filter(p => !p.ok).map(p => p.error))] }
    result.workerLinkConnected = (probes.workerLink ?? []).every(s => s.connected === true)
    result.workerDuringStall = probes.workerDuringStall ? { samples: probes.workerDuringStall.length, allConnected: probes.workerDuringStall.every(s => s.connected === true), maxMs: Math.max(...probes.workerDuringStall.map(s => s.ms ?? 0)) } : null
    const finalStatus = {}
    for (const r2 of [worker, home]) finalStatus[r2.name] = await r2.rpc({ RelayStatus: null }).then(s => s.RelayStatus.status.connected, () => null)
    result.finalConnected = finalStatus
    writeFileSync(join(evidence, "probes.json"), JSON.stringify({ small: probes.small, list: probes.list }) + "\n")
    for (const r2 of [worker, home]) {
      const logs = join(r2.dir, "logs")
      const text = readdirSync(logs).sort().map(f => readFileSync(join(logs, f), "utf8")).join("")
      writeFileSync(join(evidence, `${r2.name}.klog`), text, { mode: 0o600 })
    }
    // Lines of the relay and kernel logs that mean a connection was lost or a request ran out of time.
    const interesting = /relay_peer_removed|end_reason|read failed|did not refresh|timed out|timeout|reconnect|disconnect|capacity|closed|error/i
    const noise = /relay_connection_closed.*normal|"level":"info"/i
    for (const name of ["relay", "worker", "home"]) {
      // Only what happened from the start of the transfer: setup closes and reconnects are the harness's own.
      const after = l => { const m = /"timestamp_ms":(\d+)/.exec(l); return !m || Number(m[1]) >= startedAt }
      const text = name === "relay" ? readFileSync(join(evidence, "relay.log")).subarray(relayLogFrom).toString("utf8") : readFileSync(join(evidence, `${name}.klog`), "utf8")
      const lines = text.split("\n").filter(l => interesting.test(l) && !noise.test(l) && after(l))
      result.errors.push({ source: name, count: lines.length, first: lines.slice(0, 4).map(l => l.slice(0, 260)) })
    }
    // Presence heartbeats of the serving kernel as logged by the kernel itself.
    const presence = readFileSync(join(evidence, "worker.klog"), "utf8").split("\n").filter(l => l.includes("kernel cloud presence visible"))
    const stamp = l => { const m = /"timestamp_ms":(\d+)/.exec(l); return m ? Number(m[1]) : NaN }
    const times = presence.map(stamp).filter(Number.isFinite)
    result.presence = { lines: presence.length, maxGapS: times.length > 1 ? Math.max(...times.slice(1).map((t, i) => (t - times[i]) / 1000)) : null }
    // The probes themselves cost the downlink bytes (a kernel-list renewal is a whole relay exchange): take their idle cost off.
    const probePairs = Math.min(probes.small.length, probes.list.length)
    result.wire = { direction, downBytes: wireTo[direction === "put" ? "up" : "down"] - wireFrom[direction === "put" ? "up" : "down"], probePairBytes: Math.round(probeWireDown), probePairs }
    result.wireDownRateRaw = result.wire.downBytes / (windowMs / 1000) / bytesPerSecondLink
    result.wireDownRate = (result.wire.downBytes - probeWireDown * probePairs) / (windowMs / 1000) / bytesPerSecondLink
    // File bytes that arrived intact over the bytes the relay put on the shaped link for them: the cost of the framing and encoding.
    result.fileBytes = result.transfers.filter(t => t.code === 0 && t.hashOk).length * sizeBytes
    result.fileShare = result.fileBytes / Math.max(1, result.wire.downBytes - probeWireDown * probePairs)
  } catch (error) {
    result.fatal = error.message
  } finally {
    for (const client of clients) await client.close().catch(() => {})
    const stopped = await Promise.allSettled(children.toReversed().map(stop))
    for (const f of filesToRemove) { try { unlinkSync(f) } catch {} }
    const failed = stopped.some(s => s.status === "rejected")
    if (!failed) rmSync(root, { recursive: true, force: true })
    result.cleanup = { removed: !failed, live: children.filter(c => c.exitCode === null && c.signalCode === null).map(c => c.pid) }
    writeFileSync(join(evidence, "result.json"), JSON.stringify(result, null, 2) + "\n")
    console.log(JSON.stringify({ variant, profile: profileName, fatal: result.fatal ?? null, transfers: result.transfers, small: result.probes?.small, list: result.probes?.list }))
  }
}

mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
for (const variant of variants) await runVariant(variant)

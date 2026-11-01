// A followed target that ended before the home kernel could install it must not stop the other targets of
// that kernel from being followed: real kernels, scoped relay and compiled CLI.
// The relay is frozen (SIGSTOP) while the follower follows a second session, which then ends on the target;
// afterwards a deadline fires on the first session's agent and its deadline_reached must reach the follower.
// Credentials are generated for this disposable pair and never written to evidence.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHmac, randomBytes, randomUUID, createHash } from "node:crypto"
import { mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join, dirname } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { DatabaseSync } from "node:sqlite"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) {
  const key = process.argv[i]
  flags.set(key, process.argv[++i])
}
function required(key) {
  const value = flags.get(key)
  assert.equal(typeof value, "string", `${key} is required`)
  return resolve(value)
}
const binary = required("--kernel-binary"), cliBinary = required("--cli-binary"), relayBinary = required("--relay-binary")
const label = flags.get("--label") ?? "run"
const stateRoot = required("--state-root"), evidence = required("--evidence-dir")
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state must be lane-owned")
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const root = mkdtempSync(join(stateRoot, "outside-health-"))
const children = [], clients = [], steps = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(`MP-08/09/10/11 ${name}`)
}
const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash" }
const secret = randomBytes(32).toString("base64url"), issuer = `outside-health-${randomUUID()}`
function token(subject, kind, actions) {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer,
    allowed_actions: actions, allowed_targets: null, iat: Math.floor(Date.now()/1000)-1,
    exp: Math.floor(Date.now()/1000)+1800, jti: randomUUID(), user_id: "local" })).toString("base64url")
  const input = `${head}.${body}`
  return `${input}.${createHmac("sha256", secret).update(input).digest("base64url")}`
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(check, description, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(100) }
  throw new Error(`timeout: ${description}`)
}
function start(program, extra, label) {
  const log = openSync(join(evidence, `${label}.log`), "a", 0o600)
  const child = spawn(program, [], { env: { ...env, ...extra }, stdio: ["ignore", log, log] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1)
  children.push(child)
  note("process-start", { label, pid: child.pid, binary: program })
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("invalid owned PID")
  child.kill("SIGTERM")
  try {
    await until(() => child.exitCode !== null || child.signalCode !== null, `owned PID ${child.pid} exits`, 30000)
  } catch {
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("invalid owned PID")
    child.kill("SIGKILL")
    await until(() => child.exitCode !== null || child.signalCode !== null, `owned PID ${child.pid} exits after kill`, 5000)
  }
  note("process-exit", { pid: child.pid, code: child.exitCode, signal: child.signalCode })
}
async function role(name) {
  const dir = join(root, name), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"),
    XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"), CHARIOX_DAEMON_ALIAS: `extev-${name}`, CHARIOX_MACHINE_ALIAS: `extev-${name}`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  const r = { dir, env: localEnv, passkey: randomBytes(24).toString("hex") }
  r.child = start(binary, localEnv, name)
  r.client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 1000 })
  clients.push(r.client)
  r.rpc = async request => {
    const result = await r.client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code}`)
    return result
  }
  await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${name} ready`)
  r.status = (await r.rpc({ RelayStatus: null })).RelayStatus.status
  note("kernel-ready", { role: name, kernel: r.status.daemon_id })
  return r
}
async function unlock(r) {
  const made = (await r.rpc({ CreateSession: { workspace_id: join(r.dir, "work"), worktree_id: join(r.dir, "work"), alias: "health-drill-owner" } })).SessionCreated
  const att = (await r.rpc({ AttachToSession: { session_id: made.session.id, client_id: "health-drill", capability_level: "FullTerminal" } })).SessionAttached
  await r.client.subscribeToKernelEvents(made.session.id, att.attachment.id)
  const pending = r.rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } })
  pending.catch(() => {})
  let interaction
  await until(async () => {
    const st = (await r.rpc({ GetSessionState: { session_id: made.session.id } })).SessionState.session
    interaction = st.active_interactions?.find(x => x.id.startsWith("vault-unlock-"))
    return interaction
  }, "disposable vault prompt")
  await r.rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: interaction.id, choice_id: "passphrase", custom_reply: r.passkey } })
  await pending
  r.client.onKernelEvent(async event => {
    if (event.event !== "passkey_prompts_changed") return
    for (const prompt of event.prompts || []) if (prompt.kind === "access_grant") {
      await r.rpc({ RespondToInteraction: { session_id: prompt.session_id || "kernel-access", interaction_id: prompt.interaction_id, choice_id: "approve", passkey: r.passkey } }).catch(() => note("access-approval-failed"))
    }
  })
}
let commandSequence = 0
function cli(r, args, input) {
  const n = ++commandSequence, started = Date.now()
  const child = spawn(cliBinary, args, { env: r.env, stdio: ["pipe", "pipe", "pipe"] })
  children.push(child)
  let out = "", err = ""
  child.stdout.on("data", b => out += b)
  child.stderr.on("data", b => err += b)
  child.stdin.on("error", () => {})
  child.stdin.end(input === undefined ? undefined : JSON.stringify(input))
  return new Promise((resolve, reject) => {
    child.on("error", reject)
    child.on("close", code => {
      writeFileSync(join(evidence, `cli-${n}.stdout`), out, { mode: 0o600 })
      writeFileSync(join(evidence, `cli-${n}.stderr`), err, { mode: 0o600 })
      let value
      try { value = JSON.parse(out) } catch {}
      note("cli", { n, args, code, ms: Date.now()-started })
      resolve({ code, value, err, ms: Date.now()-started })
    })
  })
}

function signal(child, name) {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("invalid owned PID")
  child.kill(name)
}
const failures = []
function check(name, ok, detail = {}) {
  note(ok ? "check-pass" : "check-FAIL", { check: name, ...detail })
  if (!ok) failures.push(name)
}
try {
  note("binaries", Object.fromEntries([binary, cliBinary, relayBinary].map(p => [p, createHash("sha256").update(readFileSync(p)).digest("hex")])))
  const relayPort = await freePort(), relayUrl = `ws://127.0.0.1:${relayPort}`
  mkdirSync(join(root, "relay"), { mode: 0o700 })
  const relay = start(relayBinary, { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
  const home = await role("home"), worker = await role("worker")
  for (const r of [home, worker]) {
    const relayToken = token(r.status.daemon_id, "kernel", ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event"])
    await r.rpc({ ConfigureRelay: { relay_url: relayUrl, relay_token: relayToken } })
    await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, "relay registration")
  }
  const pairing = (await worker.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  await home.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("stale-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
  note("paired")
  await unlock(home)
  let r = await cli(home, ["access", "request", "--until-exit", "--holder-pid", String(process.pid)])
  assert.equal(r.code, 0, r.err)
  const target = worker.status.daemon_id
  const remote = async body => {
    const r = await cli(home, ["request", "--kernel", target, "--command-id", randomUUID()], body)
    assert.equal(r.code, 0, r.err)
    assert(!r.value.Error)
    return r.value
  }
  const newSession = async alias => (await remote({ CreateSession: { workspace_id: join(worker.dir, "work"), worktree_id: join(worker.dir, "work"), alias, no_subscribe: true } })).SessionCreated.session.id
  const lane = await newSession("stale-lane"), doomed = await newSession("stale-doomed")
  const agent = (await remote({ SpawnAgent: { session_id: lane, provider: "codex", alias: null, model: null, effort: null, execution_mode: null, permission_level: null, worktree_id: null, kernel_ref: null, slice_ref: null, worktree_placement: null, no_subscribe: true } })).AgentSpawned.agent.id
  for (const extra of [[], ["--agent", agent]]) {
    r = await cli(home, ["events", "subscribe", "--kernel", target, "--session", lane, ...extra])
    assert.equal(r.code, 0, r.err)
  }
  r = await cli(home, ["events", "wait", "--since", "0", "--timeout", "2"])
  let cursor = r.value.next_cursor
  const collected = []
  async function drain(seconds) {
    const end = Date.now() + seconds * 1000
    while (Date.now() < end) {
      const batch = await cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "2"])
      assert([0, 5].includes(batch.code), batch.err)
      collected.push(...batch.value.events)
      cursor = batch.value.next_cursor
    }
  }
  await drain(3)
  collected.length = 0

  // The follower follows a session while the relay cannot carry the install; the session ends on the target meanwhile.
  // The install request outlives its 30 s deadline, so the target stays in the follower's part without being installed.
  signal(relay, "SIGSTOP")
  let following
  try {
    following = cli(home, ["events", "subscribe", "--kernel", target, "--session", doomed])
    await delay(2000)
    await worker.rpc({ EndSession: { session_id: doomed } })
    await delay(34000)
  } finally { signal(relay, "SIGCONT") }
  r = await following
  note("subscribed-while-cut", { code: r.code, error: r.err.slice(0, 200) })
  await delay(3000)

  const deadlineAt = Date.now() + 4000
  await remote({ SetAgentDeadline: { session_id: lane, agent_id: agent, deadline_ms: deadlineAt } })
  await drain(25)
  const reached = collected.find(e => e.type === "agent" && e.value?.reason === "deadline_reached")
  const health = collected.filter(e => e.type === "kernel_health").map(e => e.value)
  check("the deadline_reached of a followed agent arrives although another followed session ended before it was installed", !!reached, { health, types: collected.map(e => e.type) })
  const ended = collected.find(e => e.type === "session" && e.session_id === doomed && e.value?.state === "ended")
  check("the follower is told the session it could not install ended", !!ended, { health })
  r = await cli(home, ["events", "subscription"])
  note("subscription", { code: r.code, err: r.err.slice(0, 300), value: r.value })
  const subscription = r.value?.subscription ?? r.value
  check("the ended session is no longer a target", r.code === 0 && Array.isArray(subscription?.targets) && !subscription.targets.some(t => t.session_id === doomed), { code: r.code })
  assert.equal((await cli(home, ["events", "unsubscribe", "--all"])).code, 0)
  await remote({ EndSession: { session_id: lane } })
  note(failures.length ? `${label}: RED ${failures.join("; ")}` : `${label}: PASS`)
  if (failures.length) process.exitCode = 1
} catch (error) {
  note("FAIL", { message: error.message }); process.exitCode = 1
} finally {
  for (const client of clients) await client.close().catch(() => {})
  const stopped = await Promise.allSettled(children.toReversed().map(stop))
  const failed = stopped.some(result => result.status === "rejected")
  if (!failed) rmSync(root, { recursive: true, force: true })
  else process.exitCode = 1
  note("cleanup", { stateRemoved: !failed, liveChildren: children.filter(c => c.exitCode === null && c.signalCode === null).map(c => c.pid) })
}

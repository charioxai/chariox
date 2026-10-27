// MP-08 / MP-09 / MP-10 / MP-11: real kernels, scoped relay and compiled CLI.
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
  if (["--expect-base", "--review-regressions", "--expect-review-base", "--round3-regressions", "--expect-round3-base"].includes(key)) flags.set(key, true)
  else flags.set(key, process.argv[++i])
}
function required(key) {
  const value = flags.get(key)
  assert.equal(typeof value, "string", `${key} is required`)
  return resolve(value)
}
const binary = required("--kernel-binary"), cliBinary = required("--cli-binary"), relayBinary = required("--relay-binary")
const manifest = JSON.parse(readFileSync(required("--binary-manifest"), "utf8"))
assert.equal(manifest.expectation, flags.has("--expect-base") ? "base" : "green")
for (const [kind, path] of [["kernel", binary], ["cli", cliBinary], ["relay", relayBinary]]) {
  assert.match(manifest[kind].head, /^[a-f0-9]{40}$/)
  assert.equal(createHash("sha256").update(readFileSync(path)).digest("hex"), manifest[kind].sha256, `${kind} differs from the recorded build`)
}
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
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
    ...(flags.has("--review-regressions") && name === "home" ? { CHARIOX_TEST_OUTSIDE_PENDING_LIMIT: "4" } : {}) }
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
async function smoke(r, target) {
  const script = required("--smoke-script")
  const child = spawn("bash", [script], { env: { ...r.env, C3_KERNEL: target,
    PATH: `${dirname(cliBinary)}:${dirname(script)}:${r.env.PATH}` },
    stdio: ["ignore", openSync(join(evidence, "smoke.stdout"), "w", 0o600), openSync(join(evidence, "smoke.stderr"), "w", 0o600)] })
  children.push(child)
  const code = await new Promise((resolve, reject) => { child.on("error", reject); child.on("close", resolve) })
  assert.equal(code, 0, "c3 finite command/file smoke failed; see smoke.stderr")
  note("c3-smoke", { code })
}
try {
  note("binaries", Object.fromEntries([binary, cliBinary, relayBinary].map(p => [p, createHash("sha256").update(readFileSync(p)).digest("hex")])))
  note("build-heads", { kernel: manifest.kernel.head, cli: manifest.cli.head, relay: manifest.relay.head })
  const relayPort = await freePort(), relayUrl = `ws://127.0.0.1:${relayPort}`
  mkdirSync(join(root, "relay"), { mode: 0o700 })
  start(relayBinary, { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
  const home = await role("home"), worker = await role("worker")
  for (const r of [home, worker]) {
    const relayToken = token(r.status.daemon_id, "kernel", ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event"])
    await r.rpc({ ConfigureRelay: { relay_url: relayUrl, relay_token: relayToken } })
    await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, "relay registration")
  }
  const pairing = (await worker.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  await home.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("health-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
  note("paired")
  await unlock(home)
  let r = await cli(home, ["access", "request", "--until-exit", "--holder-pid", String(process.pid)])
  assert.equal(r.code, 0, r.err)
  const target = worker.status.daemon_id
  const sourceEvents = new Map()
  function rememberSourceCursors() {
    // Imported payloads retain source cursors; the CLI presents home cursors.
    // Read only redacted terminal records, never kernel configuration or secrets.
    const db = new DatabaseSync(join(home.dir, "chariox/state.db"), { readOnly: true })
    try {
      const rows = db.prepare("SELECT payload FROM agent_inbox WHERE room_id='' AND json_extract(payload,'$.type')='terminal'").all()
      for (const row of rows) {
        const e = JSON.parse(row.payload)
        if (e.kernel === target) sourceEvents.set(e.refs.terminal_id, e)
      }
    } finally { db.close() }
  }
  if (flags.has("--smoke-script")) await smoke(home, target)
  const remote = async body => {
    const r = await cli(home, ["request", "--kernel", target, "--command-id", randomUUID()], body)
    assert.equal(r.code, 0, r.err)
    assert(!r.value.Error)
    return r.value
  }
  const made = (await remote({ CreateSession: { workspace_id: join(worker.dir, "work"), worktree_id: join(worker.dir, "work"), alias: "health-target", no_subscribe: true } })).SessionCreated
  const session = made.session.id
  async function refusedSubscription(existing) {
    const before = await cli(home, ["events", "list"])
    const since = before.value?.next_cursor || 0
    const refused = await cli(home, ["events", "subscribe", "--kernel", target, "--session", randomUUID()])
    assert.notEqual(refused.code, 0, "missing remote target must be refused")
    await delay(600)
    const after = await cli(home, ["events", "wait", "--since", String(since), "--timeout", "0"])
    const health = after.value.events.filter(e => e.type === "kernel_health")
    if (flags.has("--expect-review-base")) {
      assert(health.some(e => !e.value.online), "review base must reproduce the false offline event")
    } else {
      assert.equal(after.code, 5, "a refusal must leave the inbox idle")
      assert.equal(health.length, 0, "a refusal must not change kernel health")
    }
    note("refused-subscription", { existing, code: refused.code, health })
    // Drain the red-only false events before checking genuine disconnects.
    assert.equal((await cli(home, ["events", "ack", String(after.value.next_cursor)])).code, 0)
  }
  if (flags.has("--review-regressions")) await refusedSubscription(false)
  r = await cli(home, ["events", "subscribe", "--kernel", target, "--session", session])
  assert.equal(r.code, 0, r.err)
  assert.equal(r.value.subscription.kernels[0].online, true)
  if (flags.has("--review-regressions")) await refusedSubscription(true)
  r = await cli(home, ["events", "wait", ...(flags.has("--review-regressions") ? [] : ["--since", "0"]), "--timeout", "0"])
  assert([0, 5].includes(r.code), r.err)
  let cursor = r.value.next_cursor
  // Establish ordering relative to an ordinary user-visible terminal exit.
  const terminalWait = cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "10"])
  await delay(200)
  const terminal = await cli(home, ["terminal", "open", "--kernel", target, "--session", session, "--", "/bin/sh", "-c", "exit 23"])
  assert.equal(terminal.code, 0, terminal.err)
  r = await terminalWait
  assert.equal(r.code, 0, r.err)
  assert(r.value.events.some(e => e.type === "terminal" && e.value.exit_code === 23))
  if (flags.has("--review-regressions")) rememberSourceCursors()
  cursor = r.value.next_cursor
  const listedTerminalCursor = cursor
  await cli(home, ["terminal", "close", "--kernel", target, "--session", session, "--terminal", terminal.value.terminal.terminal_id])
  r = await cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "0"])
  cursor = r.value.next_cursor
  note("terminal-event", { cursor })
  const offlineStarted = Date.now()
  const offline = cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "10"])
  await delay(250)
  await stop(worker.child)
  r = await offline
  const offlineMs = Date.now() - offlineStarted
  const offlineHealth = await cli(home, ["events", "subscription"])
  assert.equal(offlineHealth.value.subscription.kernels[0].online, false)
  if (flags.has("--expect-base")) {
    assert.equal(r.code, 5, r.err)
    assert.equal(r.value.timed_out, true)
    assert.equal(r.value.events.length, 0)
  } else {
    assert.equal(r.code, 0, r.err)
    assert.equal(r.value.timed_out, false)
    assert.equal(r.value.events.length, 1)
    assert.equal(r.value.events[0].type, "kernel_health")
    assert.equal(r.value.events[0].kernel, target)
    assert.equal(r.value.events[0].value.online, false)
    assert(r.value.next_cursor > cursor)
    assert(offlineMs < 8000, `disconnect took ${offlineMs} ms`)
  }
  note("offline-wait", { ms: offlineMs, code: r.code, batch: r.value })
  cursor = r.value.next_cursor
  const recoveryStarted = Date.now()
  const recovery = cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "10"])
  await delay(250)
  worker.child = start(binary, worker.env, "worker-recovered")
  await until(async () => { try { return (await worker.rpc({ RelayStatus: null })).RelayStatus.status.connected } catch { return false } }, "worker recovery")
  r = await recovery
  const recoveryHealth = await cli(home, ["events", "subscription"])
  assert.equal(recoveryHealth.value.subscription.kernels[0].online, true)
  if (flags.has("--expect-base")) {
    assert.equal(r.code, 5, r.err)
    assert.equal(r.value.events.length, 0)
  } else {
    assert.equal(r.code, 0, r.err)
    assert.equal(r.value.timed_out, false)
    assert.equal(r.value.events.length, 1)
    assert.equal(r.value.events[0].type, "kernel_health")
    assert.deepEqual(r.value.events[0].value, { online: true, reason: "connected" })
    assert(r.value.next_cursor > cursor)
    assert(Date.now() - recoveryStarted < 8000)
  }
  note("recovery-wait", { ms: Date.now() - recoveryStarted, code: r.code, batch: r.value })
  cursor = r.value.next_cursor
  if (flags.has("--review-regressions")) {
    async function retainedEvents() {
      let batch = await cli(home, ["events", "list"])
      for (let retry = 0; batch.code === 7 && retry < 8; retry++) {
        const failure = JSON.parse(batch.err).error
        assert.equal(failure.code, "cursor_expired")
        batch = await cli(home, ["events", "list", "--since", String(failure.oldest_cursor - 1)])
      }
      assert.equal(batch.code, 0, batch.err)
      return batch
    }
    for (let i = 0; i < 5; i++) {
      const opened = await cli(home, ["terminal", "open", "--kernel", target, "--session", session, "--", "/bin/sh", "-c", `exit ${30 + i}`])
      assert.equal(opened.code, 0, opened.err)
      await until(async () => {
        const batch = await retainedEvents()
        return batch.value?.events.some(e => e.type === "terminal" && e.value.exit_code === 30 + i)
      }, "terminal imported before next overflow")
      rememberSourceCursors()
      if (flags.has("--round3-regressions") && i === 1) {
        const resumed = await cli(home, ["events", "wait", "--since", String(listedTerminalCursor), "--timeout", "0"])
        const covered = resumed.code === 7 || resumed.value?.events.some(e => e.type === "gap" && e.kernel === target)
        assert.equal(!!covered, !flags.has("--expect-round3-base"), "a dropped health transition after the listed cursor needs a gap or expiry")
        note("health-cursor-coverage", { listedTerminalCursor, covered: !!covered, code: resumed.code, batch: resumed.value })
      }
      assert.equal((await cli(home, ["terminal", "close", "--kernel", target, "--session", session, "--terminal", opened.value.terminal.terminal_id])).code, 0)
    }
    const merged = await retainedEvents()
    const gap = merged.value.events.find(e => e.type === "gap" && e.kernel === target)
    assert(gap, "overflow must retain a source gap")
    const retained = merged.value.events.filter(e => e.type === "terminal").map(e => e.refs.terminal_id)
    const source = [...sourceEvents.values()].sort((a, b) => a.cursor - b.cursor)
    assert.equal(source.length, 6, "every generated remote terminal event must have its source cursor recorded")
    const dropped = source.filter(e => !retained.includes(e.refs.terminal_id))
    assert(dropped.length > 0)
    const expected = { kernel: target, dropped: dropped.length, from: dropped[0].cursor, to: dropped.at(-1).cursor }
    if (flags.has("--expect-review-base")) assert.notDeepEqual(gap.value, expected, "review base must reproduce mixed gap metadata")
    else assert.deepEqual(gap.value, expected, "gap range and count must contain remote source events only")
    note("source-gap", { actual: gap.value, expected, source, merged: merged.value })
    cursor = merged.value.next_cursor
  }
  if (flags.has("--round3-regressions")) {
    // Restart the home pump after the followed session ends. Its reinstallation
    // is now persistently refused, while the last committed baseline is online.
    assert.equal((await cli(home, ["events", "ack", String(cursor)])).code, 0)
    await stop(home.child)
    await worker.rpc({ EndSession: { session_id: session } })
    home.child = start(binary, home.env, "home-pump-resumed")
    await until(async () => { try { return (await home.rpc({ RelayStatus: null })).RelayStatus.status.connected } catch { return false } }, "home pump restart")
    const result = await cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "2"])
    const sub = await cli(home, ["events", "subscription"])
    const issue = sub.value.subscription.kernels.find(k => k.kernel === target)
    const offline = result.value?.events.filter(e => e.type === "kernel_health" && !e.value.online) || []
    if (flags.has("--expect-round3-base")) {
      assert.equal(offline.length, 0, "base must silently suppress pump refusals")
      assert.equal(issue.error.code, "events_connecting", "base must fail to record the pump refusal in its cache")
    } else {
      assert.equal(offline.length, 1, "persistent pump refusal must wake once")
      assert.equal(issue.online, false)
      assert.equal(issue.error.code, "events_refused")
      assert.deepEqual(offline[0].value, { online: false, reason: "events_part_failed" })
    }
    note("persistent-pump-refusal", { batch: result.value, health: issue })
    cursor = result.value.next_cursor
    const again = await cli(home, ["events", "wait", "--since", String(cursor), "--timeout", "1"])
    assert.equal(again.code, 5, "unchanged persistent refusal must not wake again")
  }
  assert.equal((await cli(home, ["events", "ack", String(cursor)])).code, 0)
  assert.equal((await cli(home, ["events", "unsubscribe", "--all"])).code, 0)
  if (!flags.has("--round3-regressions")) await remote({ EndSession: { session_id: session } })
  note(flags.has("--expect-base") ? "BASE-RED-CONFIRMED" : flags.has("--expect-review-base") ? "REVIEW-BASE-RED-CONFIRMED" : flags.has("--expect-round3-base") ? "ROUND3-BASE-RED-CONFIRMED" : "PASS")
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

// Run with an isolated systemd unit and a clean environment. The kernel and relay use fresh homes,
// loopback ports and a per-run signing key. The loopback Cloud endpoint records publication and renewal.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHmac, randomBytes, randomUUID } from "node:crypto"
import { accessSync, closeSync, constants, existsSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, realpathSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:http"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { proveDeadHolderDoesNotSpin } from "./execstall-holder.mjs"
import { proveParkedPeerSubscription } from "./execstall-notifications.mjs"
import { startOwnerCloud } from "./local-owner-cloud.test-support.mjs"

const [kernelArg, relayArg, stateRoot, evidence, expected = "green", scenario = "traffic", provider = "opencode"] = process.argv.slice(2)
assert(kernelArg && relayArg && evidence && stateRoot?.startsWith("/root/.chariox/dev/"))
assert(["red", "green"].includes(expected))
assert(["traffic", "notification", "holder"].includes(scenario))
const kernelBinary = resolve(kernelArg), relayBinary = resolve(relayArg)
accessSync(kernelBinary, constants.X_OK)
accessSync(relayBinary, constants.X_OK)
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, "drill-"))
const children = [], clients = [], reports = [], renewals = []
const realm = `execstall-${randomUUID()}`, secret = randomBytes(32).toString("base64url")
const deadline = 60_000
const env = { PATH: "/usr/local/bin:/usr/bin:/bin", SHELL: "/bin/bash", IS_SANDBOX: "1" }
const wait = async (check, label, limit = 20_000) => {
  const end = Date.now() + limit
  do {
    const result = await check()
    if (result) return result
    await delay(100)
  } while (Date.now() < end)
  throw new Error(`Timed out waiting for ${label}`)
}
async function open(server) {
  await new Promise((done, fail) => {
    server.once("error", fail)
    server.listen(0, "127.0.0.1", done)
  })
  return server.address().port
}
async function freePort() {
  const server = createServer()
  const port = await open(server)
  await new Promise(done => server.close(done))
  return port
}
async function start(binary, args, extra, name) {
  const log = openSync(join(evidence, `${name}.stderr`), "a", 0o600)
  let child
  try {
    child = spawn(binary, args, { env: { ...env, ...extra }, stdio: ["ignore", log, log] })
    await new Promise((started, failed) => {
      child.once("spawn", started)
      child.once("error", failed)
    })
  } finally {
    closeSync(log)
  }
  assert(child.pid > 1, `Could not start ${name}`)
  children.push(child)
  return child
}
const exited = child => child.exitCode !== null || child.signalCode !== null
async function stop(child) {
  if (exited(child)) return
  child.kill("SIGTERM")
  try {
    await wait(() => exited(child), `process ${child.pid} to stop`, 15_000)
  } catch {
    child.kill("SIGKILL")
    await wait(() => exited(child), `process ${child.pid} to exit`, 5_000)
  }
}
function token(subject, kind, actions, target, thumbprint) {
  const h = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const now = Math.floor(Date.now() / 1000)
  const b = Buffer.from(JSON.stringify({
    iss: realm, realm_id: realm, sub: subject, subject_kind: kind, allowed_actions: actions,
    allowed_targets: target ? [target] : null, public_key_thumbprint: thumbprint,
    iat: now - 1, exp: now + (kind === "kernel" ? 72 : 900), jti: randomUUID(), user_id: "local",
  })).toString("base64url")
  return `${h}.${b}.${createHmac("sha256", secret).update(`${h}.${b}`).digest("base64url")}`
}
let cloud
try {
  const relayPort = await freePort()
  const cloudEnv = await startOwnerCloud({
    realm, relayPort,
    onServer: server => { cloud = server },
    onPresenceReport: ({ kernelId, status, startedAtMs }) => {
      reports.push({ at: Date.now(), kernel: kernelId, status, started: startedAtMs })
    },
    note: (name, fields) => {
      if (name === "owner-cloud-token") renewals.push({ at: Date.now(), kernel: fields.subject })
    },
    token: (subject, kind, actions, thumbprint) => token(subject, kind, actions, null, thumbprint),
  })
  const relayUrl = `ws://127.0.0.1:${relayPort}`
  const runRelay = () => start(relayBinary, [], {
    CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_STUN_ADDRESS: "off",
    CHARIOX_RELAY_SCOPED_ISSUER: realm, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret,
  }, "relay")
  const relay = await runRelay()
  // The real gateway startup path waits for the fixture to answer readiness. The first
  // gateway starts normally and then exits, leaving its next launch slow on purpose.
  const gateway = join(root, "gateway.cjs")
  const gatewayRestarted = join(root, "gateway-restarted")
  writeFileSync(gateway, `#!/usr/bin/node
const fs = require("node:fs")
process.on("uncaughtException", error => {
  fs.appendFileSync(${JSON.stringify(join(evidence, "gateway-errors.log"))}, String(error) + "\\n")
  process.exit(1)
})
if (process.argv[3] === "prepare") {
  if (fs.existsSync(${JSON.stringify(gatewayRestarted)})) setTimeout(() => {}, 14_000)
} else {
  fs.writeFileSync(${JSON.stringify(gatewayRestarted)}, "1")
  require("node:http").createServer((req, res) => res.end(req.url))
    .listen(Number(process.argv[6]), "127.0.0.1")
  setTimeout(() => process.exit(0), 1_500)
}
`, { mode: 0o700 })
  const holderShell = realpathSync("/bin/sh")
  const target = await bootKernel("kernel", cloudEnv, { CHARIOX_CLI_BIN: gateway, ...(scenario === "holder" ? { CHARIOX_CLAUDE_BIN: holderShell } : {}) },
    scenario === "holder" ? `[[kernel_access.standing_holders]]\nharness = "claude"\nexecutable = "${holderShell}"\n` : "")
  if (scenario === "holder") {
    const work = join(target.dir, "work")
    const room = (await target.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: "execstall" } })).SessionCreated
    await proveDeadHolderDoesNotSpin({
      target, room, socket: join(target.dir, "daemon.sock"), nodeExecutable: process.execPath,
      kernelDir: target.dir, wait, logs, evidence, expected,
    })
  } else if (scenario === "notification") {
    const source = await bootKernel("source", cloudEnv, { CHARIOX_ROOM_AGENT_TOOLS: "1" })
    const result = await proveParkedPeerSubscription({ target, source, wait, logs, provider, expected })
    writeFileSync(join(evidence, "notification-result.json"), JSON.stringify(result, null, 2))
  } else {
    await proveRelayTraffic({ target, relay, relayUrl, runRelay })
  }
} finally {
  for (const client of clients) await client.close()
  for (const child of children.reverse()) await stop(child)
  if (cloud) {
    cloud.closeAllConnections()
    await new Promise(done => cloud.close(done))
  }
  for (const label of ["kernel", "source"]) {
    const logDir = join(root, label, "logs")
    if (!existsSync(logDir)) continue
    for (const file of readdirSync(logDir)) {
      if (file.endsWith(".ndjson")) writeFileSync(join(evidence, file), readFileSync(join(logDir, file)), { mode: 0o600 })
    }
  }
  rmSync(root, { recursive: true, force: true })
}
async function send(client, request) {
  const result = await client.send(request)
  if (result.Error) throw new Error(`${Object.keys(request)[0]}: ${result.Error.message}`)
  return result
}

function logs(dir) {
  const logDir = join(dir, "logs")
  return readdirSync(logDir).filter(name => name.endsWith(".ndjson")).flatMap(name =>
    readFileSync(join(logDir, name), "utf8").trimEnd().split("\n").flatMap(line => {
      try { return [JSON.parse(line)] }
      catch { return [] }
    }))
}

async function bootKernel(label, cloudEnv, extra, configExtra = "") {
  const dir = join(root, label)
  mkdirSync(dir, { mode: 0o700 })
  const [port, mcpPort, opencodePort, codexPort] = await Promise.all(Array.from({ length: 4 }, freePort))
  for (const name of ["state", "home", "logs", "history", "tmp", "work"]) mkdirSync(join(dir, name))
  const home = join(dir, "state")
  writeFileSync(join(home, "config.toml"), `[credential_vault]
backend = "chariox_encrypted"
path = "${home}/vault.json"
unlock_policy = "kernel_init"
[state]
path = "${home}/state.db"
${configExtra}`, { mode: 0o600 })
  const kernelEnv = {
    HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), CHARIOX_HOME: home, CHARIOX_LOG_DIR: join(dir, "logs"),
    CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"), CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"),
    CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_OPENCODE_PORT: String(opencodePort),
    CHARIOX_CODEX_PORT: String(codexPort), ...cloudEnv, ...extra,
  }
  const run = () => start(kernelBinary, [], kernelEnv, label)
  const connect = () => new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1_000 })
  let child = await run()
  let client = connect()
  clients.push(client)
  return {
    dir,
    pid: () => child.pid,
    send: request => send(client, request),
    async restart() {
      await client.close()
      await stop(child)
      child = await run()
      client = connect()
      clients.push(client)
    },
  }
}

async function proveRelayTraffic({ target, relay, relayUrl, runRelay }) {
  const first = await wait(() => reports.find(r => r.status === "ONLINE"), "Cloud to see the kernel")
  assert(first.started > 0, "Presence contains the relay registration")
  const work = join(target.dir, "work")
  const room = (await target.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: "execstall" } })).SessionCreated
  const relayClient = new LocalIpcClient(relayUrl, {
    relayAuthToken: token("execstall-owner", "client", ["client.connect", "client.metadata.read", "packet.route"], first.kernel),
    targetDaemonId: first.kernel, controlRequestRetryDeadlineMs: 1_000,
  })
  clients.push(relayClient)
  await target.send({ CreateAgentTrigger: {
    session_id: room.session.id, agent_id: room.agent.id, source: { kind: "http" }, prompt: "Report the drill event",
  } })
  const startMs = Date.now()
  const samples = []
  while (Date.now() - startMs < deadline) {
    await Promise.all(Array.from({ length: 8 }, async () => {
      const start = Date.now()
      try {
        await send(relayClient, { GetSessionState: { session_id: room.session.id } })
        samples.push({ ok: true, ms: Date.now() - start })
      } catch {
        samples.push({ ok: false, ms: Date.now() - start })
      }
    }))
    await delay(100)
  }
  assert(samples.some(sample => sample.ok), "Relay load reached the kernel")
  assert(renewals.length >= 2, "The Cloud endpoint received a token renewal")
  const online = reports.filter(r => r.status === "ONLINE").map(r => r.at)
  assert(online.length >= 2, "Presence refreshed during the load")
  assert(reports.slice(reports.indexOf(first)).every(r => r.status === "ONLINE"), "The connected kernel stayed online")
  assert(Math.max(...online.slice(1).map((time, i) => time - online[i])) < 45_000, "Presence did not lapse")
  const kernelLogs = logs(target.dir)
  const pumpStalls = kernelLogs.filter(row => row.message === "kernel operation stalled" && row.operation === "transport_pump")
  assert.equal(pumpStalls.length > 0, expected === "red", "Slow gateway startup and the transport pump are independent")
  const maintenanceStalls = kernelLogs.filter(row => row.message === "kernel operation stalled" && row.operation === "transport_maintenance")
  if (expected === "green") {
    assert(maintenanceStalls.some(row => row.stage === "reconcile agent trigger gateways"), "The stall names the gateway step")
  }
  const beforeDrop = reports.length
  await stop(relay)
  await wait(() => reports.slice(beforeDrop).some(r => r.status === "OFFLINE"), "Cloud to see the relay drop")
  const beforeReconnect = reports.length
  await runRelay()
  await wait(() => reports.slice(beforeReconnect).some(r => r.status === "ONLINE"), "Cloud to see the relay reconnect")
  assert((await target.send({ RelayStatus: null })).RelayStatus.status.connected)
  await send(relayClient, { GetSessionState: { session_id: room.session.id } })
  writeFileSync(join(evidence, "result.json"), JSON.stringify({ expected, samples, reports, renewals, pumpStalls, maintenanceStalls }, null, 2))
}

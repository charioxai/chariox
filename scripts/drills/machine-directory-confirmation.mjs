// Run against a built kernel and relay on a private builder. Only the Cloud directory is simulated.
// node scripts/drills/machine-directory-confirmation.mjs <kernel> <relay> <lane-state-dir>
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHmac, randomBytes, randomUUID } from "node:crypto"
import { closeSync, mkdirSync, mkdtempSync, openSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { startOwnerCloud } from "./local-owner-cloud.test-support.mjs"

const [kernelBinary, relayBinary, stateRoot] = process.argv.slice(2).map(value => resolve(value))
assert(stateRoot?.startsWith("/root/.chariox/dev/"), "provide a private lane state directory")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, "dc-"))
const processes = []
const clients = []
const relayPort = await freePort()
const realm = randomUUID()
const secret = randomBytes(32).toString("base64url")
const machineId = "dirconfirm-peer"
const ownerMachineId = "dirconfirm-owner"
let directoryMode = "full"
let missed = 0
let failed = 0
let cloud

async function freePort() {
  const server = createServer()
  await new Promise(done => server.listen(0, "127.0.0.1", done))
  const { port } = server.address()
  await new Promise((done, fail) => server.close(error => error ? fail(error) : done()))
  return port
}

async function until(check, description) {
  const deadline = Date.now() + 30000
  while (Date.now() < deadline) {
    const value = await check()
    if (value) return value
    await delay(100)
  }
  throw new Error(`timed out waiting for ${description}`)
}

function token(subject, kind, actions, thumbprint, targets) {
  const now = Math.floor(Date.now() / 1000)
  const header = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const claims = Buffer.from(JSON.stringify({
    iss: realm, realm_id: realm, sub: subject, subject_kind: kind, user_id: "local",
    allowed_actions: actions, allowed_targets: targets ?? null, public_key_thumbprint: thumbprint,
    jti: randomUUID(), iat: now - 1, exp: now + 1800,
  })).toString("base64url")
  const input = `${header}.${claims}`
  return `${input}.${createHmac("sha256", secret).update(input).digest("base64url")}`
}

function start(binary, label, env) {
  const fd = openSync(join(root, `${label}.log`), "a", 0o600)
  try {
    const child = spawn(binary, [], {
      env: { PATH: process.env.PATH, SHELL: "/bin/sh", ...env },
      stdio: ["ignore", fd, fd],
    })
    assert(child.pid > 1, `${label} should start`)
    processes.push(child)
    return child
  } finally { closeSync(fd) }
}

async function stop(child) {
  const exited = () => child.exitCode !== null || child.signalCode !== null
  if (exited()) return
  child.kill("SIGTERM")
  try { await until(exited, `process ${child.pid} to exit`) }
  catch {
    child.kill("SIGKILL")
    await until(exited, `process ${child.pid} to be reaped`)
  }
}

function connect(env) {
  const client = new LocalIpcClient(`ws://127.0.0.1:${env.CHARIOX_KERNEL_PORT}/kernel`, {
    localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000,
  })
  clients.push(client)
  return client
}

async function rpc(client, request) {
  const name = Object.keys(request)[0]
  const timeout = new AbortController()
  try {
    const reply = await Promise.race([
      client.send(request),
      delay(20000, null, { signal: timeout.signal }).then(() => {
        throw new Error(`${name} timed out`)
      }),
    ])
    assert(!reply.Error, `${name} failed: ${reply.Error?.message}`)
    return reply
  } finally { timeout.abort() }
}

async function connected(client, child) {
  await until(async () => {
    assert.equal(child.exitCode, null, "kernel exited before connecting")
    const status = await rpc(client, { RelayStatus: null }).catch(() => null)
    return status?.RelayStatus.status.connected
  }, "kernel relay registration")
}

async function startKernel(name, profile) {
  const dir = join(root, name)
  mkdirSync(dir, { mode: 0o700 })
  const env = {
    HOME: dir, TMPDIR: dir, CHARIOX_HOME: join(dir, "chariox"),
    CHARIOX_MACHINE_ID: `dirconfirm-${name}`,
    CHARIOX_CLOUD_RELAY_CONFIG_JSON: JSON.stringify(profile),
    CHARIOX_DAEMON_SOCKET: join(dir, "kernel.sock"),
    CHARIOX_KERNEL_PORT: String(await freePort()),
    CHARIOX_MCP_PORT: String(await freePort()),
    CHARIOX_CODEX_PORT: String(await freePort()),
    CHARIOX_OPENCODE_PORT: String(await freePort()),
    CHARIOX_RELAY_HEARTBEAT_MS: "200",
  }
  const child = start(kernelBinary, name, env)
  const client = connect(env)
  await connected(client, child)
  return { child, client, env }
}

async function machines(client) {
  const { RemoteMachinesListed: list } = await rpc(client, { ListRemoteMachines: null })
  assert(!list.renewing, "the renewed list should arrive with the response")
  return list.machines
}

async function renew(client) {
  // Let the public list's one-second freshness window expire so this request waits for renewal.
  await delay(1100)
  return machines(client)
}

function peer(rows) {
  return rows.find(machine => machine.machine_id === machineId)
}

try {
  start(relayBinary, "relay", {
    HOME: root, CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort),
    CHARIOX_RELAY_SCOPED_ISSUER: realm, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret,
    CHARIOX_RELAY_STUN_ADDRESS: "off",
  })
  const config = await startOwnerCloud({
    token, realm, relayPort, note: () => {}, onServer: server => { cloud = server },
    listTargets(targets, caller) {
      if (caller !== ownerMachineId || directoryMode === "full") return targets
      if (directoryMode === "linked-offline") return targets.map(target =>
        target.machineId === machineId ? { ...target, status: "OFFLINE" } : target)
      if (directoryMode === "fail") {
        failed++
        return null
      }
      missed++
      if (directoryMode === "omit-once") directoryMode = "fail"
      return targets.filter(target => target.machineId !== machineId)
    },
  })
  const profile = JSON.parse(config.CHARIOX_CLOUD_RELAY_CONFIG_JSON)
  const owner = await startKernel("owner", { ...profile, machine_id: ownerMachineId })
  await startKernel("peer", { ...profile, machine_id: machineId })
  await until(async () => peer(await machines(owner.client))?.presence === "online", "peer in the directory-backed list")
  const approved = await rpc(owner.client, { ApproveRemoteMachine: { machine_ref: machineId } })
  assert.equal(approved.RemoteMachineApproved.machine.trust_status, "approved")

  directoryMode = "linked-offline"
  const firstList = await renew(owner.client)
  assert.equal(peer(firstList)?.presence, "reconnecting")
  assert.equal(peer(firstList)?.trust_status, "approved")
  const second = connect(owner.env)
  assert.deepEqual(await machines(second), firstList, "a second client sees the same list and approvals")
  await second.close()
  await owner.client.close()
  await stop(owner.child)
  const restarted = start(kernelBinary, "owner-restarted", owner.env)
  const client = connect(owner.env)
  await connected(client, restarted)
  const restartList = await renew(client)
  assert.deepEqual(restartList, firstList, "the list and approvals persist across a kernel restart")
  console.log("D2 PASS: second client and restarted kernel agree with the approved list")

  directoryMode = "full"
  assert.equal(peer(await renew(client))?.presence, "online")
  directoryMode = "omit-once"
  const firstOmission = peer(await renew(client))
  assert.equal(firstOmission?.presence, "reconnecting")
  assert.equal(firstOmission?.trust_status, "approved")
  assert.equal(missed, 1)
  const replays = []
  for (let i = 0; i < 3; i++) {
    const before = failed
    replays.push(peer(await renew(client))?.trust_status ?? null)
    assert(failed > before, "renewal must encounter a directory failure")
  }
  assert.equal(missed, 1, "Cloud served exactly one successful omission")
  directoryMode = "full"
  const recovered = peer(await renew(client))
  console.log(`D1: one omission, ${failed} failed directory fetches, replay trust ${JSON.stringify(replays)}, recovered trust ${recovered?.trust_status}`)
  assert.deepEqual(replays, ["approved", "approved", "approved"], "a cached omission must preserve the remembered machine and approval")
  assert.equal(recovered?.trust_status, "approved")

  directoryMode = "omit"
  assert.equal(peer(await renew(client))?.trust_status, "approved", "one fresh omission keeps the approval")
  assert.equal(peer(await renew(client)), undefined, "a second fresh omission removes the machine")
  directoryMode = "full"
  assert.equal(peer(await renew(client))?.trust_status, "pending", "confirmed removal clears the old approval")
  console.log("D1 PASS: cached omissions preserve approval and two fresh omissions remove it")
} finally {
  for (const client of clients) await client.close()
  for (const child of processes.reverse()) await stop(child)
  cloud?.closeAllConnections()
  cloud?.close()
  rmSync(root, { recursive: true, force: true })
}

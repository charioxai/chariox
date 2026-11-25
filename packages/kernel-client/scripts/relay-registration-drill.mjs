// Usage: node packages/kernel-client/scripts/relay-registration-drill.mjs RELAY KERNEL L12_RELAY L12_KERNEL
// For baseline evidence: append --baseline to a command using the L12 binaries.
// For the reply-queue status case: supply RELAY KERNEL --reply-queue.
// For a relay replaced under a refused kernel (B1, B2): RELAY KERNEL OLD_RELAY --recovery,
// where OLD_RELAY refuses the kernel (a relay older than it) and RELAY accepts it.
// Linux with systemd. All endpoints, credentials, state and units belong to this drill.
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { createHmac, randomBytes, randomUUID } from "node:crypto"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { createServer } from "node:http"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { WebSocket, WebSocketServer } from "ws"

const baseline = process.argv.includes("--baseline")
const replyQueue = process.argv.includes("--reply-queue")
const recovery = process.argv.includes("--recovery")
const [relayBinary, kernelBinary, oldRelayBinary, oldKernelBinary] =
  process.argv.slice(2).filter(arg => !["--baseline", "--reply-queue", "--recovery"].includes(arg)).map(path => resolve(path))
assert(relayBinary && kernelBinary && (baseline || replyQueue || (oldRelayBinary && (recovery || oldKernelBinary))),
  "supply the relay and kernel binaries and their L12 predecessors")
const scratch = await mkdtemp(join(tmpdir(), "chrrg-"))
const names = `relay-registration-${process.pid}`
const issuerSecret = randomBytes(32).toString("hex")
const localToken = randomBytes(32).toString("hex")
const units = []
const servers = []
const sockets = new Set()
const presence = []
const attempts = []
let releaseRegistration
let releaseAck
let closeReplyQueue
let forwardedRefusals = 0
let refusedSocketsClosed = 0

async function until(description, probe, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const value = await probe()
    if (value) return value
    await delay(100)
  }
  throw new Error(`Timed out waiting for ${description}`)
}

async function listen(server) {
  servers.push(server)
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  return server.address().port
}

async function freePort() {
  const server = createServer()
  const port = await listen(server)
  await new Promise(resolve => server.close(resolve))
  return port
}

function runUnit(label, program, env) {
  const unit = `${names}-${label}.service`
  execFileSync("systemd-run", [
    "--quiet", "--collect", "--unit", unit, "--property=RuntimeMaxSec=900",
    "/usr/bin/env", "-i", "PATH=/usr/bin:/bin", "SHELL=/bin/sh",
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`), program,
  ])
  units.push(unit)
  return unit
}

function track(socket) {
  sockets.add(socket)
  socket.on("close", () => sockets.delete(socket))
  return socket
}

function open(url, headers) {
  return new Promise((resolve, reject) => {
    const socket = track(new WebSocket(url, { headers, handshakeTimeout: 1500 }))
    socket.once("error", reject)
    socket.once("open", () => resolve(socket))
  })
}

function request(socket, body) {
  return new Promise((resolve, reject) => {
    const id = randomUUID()
    const timer = setTimeout(() => {
      socket.off("message", received)
      reject(new Error(`No response to ${Object.keys(body)[0]}`))
    }, 3000)
    function received(bytes) {
      const frame = JSON.parse(bytes.toString())
      if (frame.type !== "response" || frame.request_id !== id) return
      clearTimeout(timer)
      socket.off("message", received)
      if (frame.error) reject(new Error(`${Object.keys(body)[0]} was refused`))
      else resolve(frame.response)
    }
    socket.on("message", received)
    socket.send(JSON.stringify({ type: "request", request_id: id, command_id: id, request: body }))
  })
}

// A local issuer and presence receiver. The actual kernel decides when to publish.
async function cloud(req, res) {
  const chunks = []
  for await (const chunk of req) chunks.push(chunk)
  const body = JSON.parse(Buffer.concat(chunks).toString() || "{}")
  res.setHeader("Content-Type", "application/json")
  if (req.url === "/kernels/presence") {
    presence.push({ kernel: body.kernelId, status: body.status, time: Date.now(), relayDisconnect: body.metadata?.relay_disconnect })
    res.end("{}")
  } else if (req.url === "/relay/token" && body.subjectKind === "kernel") {
    const now = Math.floor(Date.now() / 1000)
    const exp = now + 1800
    const json64 = value => Buffer.from(JSON.stringify(value)).toString("base64url")
    const claims = json64({
      iss: "relay-registration-drill", sub: body.subject, subject_kind: "kernel",
      realm_id: body.realmId, account_id: body.accountId, user_id: body.userId,
      machine_id: body.machineId, public_key_thumbprint: body.publicKeyThumbprint,
      allowed_actions: ["daemon.register", "daemon.heartbeat"],
      iat: now, exp, jti: randomUUID(),
    })
    const signed = `${json64({ alg: "HS256", typ: "JWT" })}.${claims}`
    const signature = createHmac("sha256", issuerSecret).update(signed).digest("base64url")
    res.end(JSON.stringify({ token: `${signed}.${signature}`, expiresAt: new Date(exp * 1000).toISOString() }))
  } else {
    res.statusCode = 404
    res.end("{}")
  }
}

// Forward the handshake and all traffic to the real relay. Only registration
// frames are changed, and only for the isolated connections under test.
async function relayProxy(port, mode) {
  const websocket = new WebSocketServer({ noServer: true })
  const server = createServer()
  const responseHeaders = new WeakMap()
  websocket.on("headers", (headers, req) => {
    for (const [name, value] of Object.entries(responseHeaders.get(req))) {
      if (name.startsWith("x-chariox-")) headers.push(`${name}: ${value}`)
    }
  })
  server.on("upgrade", (req, raw, head) => {
    const headers = Object.fromEntries(Object.entries(req.headers).filter(([name]) =>
      name === "host" || name.startsWith("x-chariox-")))
    const upstream = track(new WebSocket(`ws://127.0.0.1:${port}${req.url}`, { headers }))
    upstream.on("error", () => raw.destroy())
    upstream.on("upgrade", response => responseHeaders.set(req, response.headers))
    upstream.once("open", () => websocket.handleUpgrade(req, raw, head, downstream => {
      track(downstream).on("error", () => upstream.terminate())
      let sentRegistration = false
      downstream.on("message", (bytes, binary) => {
        if (binary || mode !== "refuse") {
          upstream.send(bytes, { binary })
          return
        }
        const frame = JSON.parse(bytes.toString())
        if (frame.kind !== "daemon_register") {
          upstream.send(bytes, { binary })
          return
        }
        attempts.push(Date.now())
        frame.registration.local_protocol = 1011
        delete frame.proof
        const send = () => {
          sentRegistration = true
          upstream.send(JSON.stringify(frame))
        }
        if (attempts.length === 1) releaseRegistration = send
        else send()
      })
      upstream.on("message", (bytes, binary) => {
        const text = binary ? "" : bytes.toString()
        if (mode === "reply-queue" && text === '{"kind":"daemon_registered"}' && !closeReplyQueue) {
          closeReplyQueue = () => downstream.close(1013, "try again later")
          downstream.send(text)
        } else if ((mode === "ack" || mode === "reply-queue") && text === '{"kind":"daemon_registered"}') {
          releaseAck = () => downstream.send(text)
        } else {
          if (mode === "refuse" && text.startsWith('{"kind":"close"')) forwardedRefusals++
          downstream.send(bytes, { binary })
        }
      })
      upstream.on("close", () => {
        if (sentRegistration) refusedSocketsClosed++
        downstream.close()
      })
      downstream.on("close", () => upstream.close())
    }))
  })
  return `ws://127.0.0.1:${await listen(server)}`
}

async function startKernel(label, relay, cloudPort, binary = kernelBinary) {
  const kernelId = `relay-registration-${label}`
  const home = join(scratch, label)
  await mkdir(home, { recursive: true })
  const port = await freePort()
  const unit = runUnit(label, binary, {
    HOME: home, CHARIOX_HOME: join(home, ".chariox"), CHARIOX_LOG_DIR: join(home, "logs"),
    CHARIOX_KERNEL_PORT: port, CHARIOX_MCP_PORT: await freePort(), CHARIOX_DAEMON_SOCKET: join(home, "k.sock"),
    CHARIOX_DAEMON_ID: kernelId, CHARIOX_MACHINE_ID: kernelId, CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: localToken,
    CHARIOX_CLOUD_RELAY_CONFIG_JSON: JSON.stringify({
      api_url: `http://127.0.0.1:${cloudPort}`, relay_url: relay, issuer_id: "relay-registration-drill",
      email: "drill@example.test", account_id: "drill", user_id: "drill", account_slug: "drill", realm_id: "drill",
      machine_id: kernelId, kernel_id: kernelId, kernel_credential: randomUUID(),
    }),
  })
  const socket = await until(`kernel ${label} endpoint`, () =>
    open(`ws://127.0.0.1:${port}/kernel`, { authorization: `Bearer ${localToken}` }).catch(() => null))
  return { kernelId, unit, home, socket }
}

const status = async kernel => (await request(kernel.socket, { RelayStatus: null })).RelayStatus.status
const online = kernel => presence.some(event => event.kernel === kernel.kernelId && event.status === "ONLINE")
const notices = async (kernel, session, attachment) =>
  (await request(kernel.socket, {
    PollRuntimeNotices: { session_id: session, attachment_id: attachment },
  })).RuntimeNotices.notices.filter(notice => notice.message.includes("Relay not registered:"))

async function startRelay(label, binary, port) {
  const relayPort = port ?? await freePort()
  runUnit(label, binary, {
    CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: relayPort, CHARIOX_RELAY_STUN_ADDRESS: "off",
    CHARIOX_RELAY_SCOPED_ISSUER: "relay-registration-drill", CHARIOX_RELAY_SCOPED_HMAC_SECRET: issuerSecret,
  })
  await until("relay readiness", () =>
    fetch(`http://127.0.0.1:${relayPort}/healthz`).then(response => response.ok).catch(() => false))
  return relayPort
}

// B1, B2: a kernel refused by a relay older than itself registers within a minute of the
// relay being replaced by one that accepts it, however long it was refused, and a relay
// that keeps refusing is dialed at a bounded rate with one warning.
async function recoveryDrill() {
  const cloudPort = await listen(createServer(cloud))
  const relayPort = await freePort()
  const oldRelayUnit = `${names}-old-relay.service`
  await startRelay("old-relay", oldRelayBinary, relayPort)
  const kernel = await startKernel("recovery", `ws://127.0.0.1:${relayPort}`, cloudPort)
  const started = Date.now()
  const refusals = []
  let observed
  await until("three refusals from the old relay", async () => {
    const current = (await status(kernel)).registration_refused
    if (current && current.next_retry_at_ms !== observed) {
      observed = current.next_retry_at_ms
      refusals.push({ at: Date.now(), reason: current.reason, retry_in_ms: current.next_retry_at_ms - Date.now() })
    }
    return refusals.length >= 3
  }, 240_000)
  assert.equal(online(kernel), false)
  const gaps = refusals.slice(1).map((refusal, index) => refusal.at - refusals[index].at)
  console.log("refusals", JSON.stringify({ refusals, gaps }))
  // B2: the retry waits at least the first step, so no tight loop.
  assert(gaps.every(gap => gap >= 14_000), `refusals ${JSON.stringify(gaps)} ms apart`)
  const logged = spawnSync("sh", ["-c", `cat ${join(kernel.home, "logs")}/* 2>/dev/null`], { encoding: "utf8", maxBuffer: 1 << 28 }).stdout
  const scheduled = logged.split("\n").filter(line => line.includes("relay registration retry scheduled")).length
  const warned = logged.split("\n").filter(line => line.includes("Relay not registered")).length
  assert(scheduled >= refusals.length && scheduled <= refusals.length + 1, `${scheduled} retry lines for ${refusals.length} refusals`)
  assert(warned <= 2, `${warned} refusal warnings`)
  console.log("B2 PASS", JSON.stringify({ attempts: refusals.length, gaps, scheduled, warned }))

  execFileSync("systemctl", ["stop", oldRelayUnit])
  await startRelay("relay", relayBinary, relayPort)
  const replaced = Date.now()
  const registered = await until("kernel registered with the replacement relay", () =>
    presence.find(event => event.kernel === kernel.kernelId && event.status === "ONLINE" && event.time >= replaced), 120_000)
  const waited = registered.time - replaced
  assert.equal((await status(kernel)).connected, true)
  assert(waited < 60_000, `registered ${waited} ms after the relay was replaced`)
  console.log("B1 PASS", JSON.stringify({ since_start_ms: replaced - started, waited_ms: waited }))
}

async function drill() {
  if (recovery) return recoveryDrill()
  const cloudPort = await listen(createServer(cloud))
  const relayPort = await startRelay("relay", relayBinary)
  if (replyQueue) {
    const kernel = await startKernel("reply-queue", await relayProxy(relayPort, "reply-queue"), cloudPort)
    await until("kernel registered with the relay", () => online(kernel))
    const dropIndex = presence.length
    closeReplyQueue()
    const offline = await until("kernel reported the dropped connection", () =>
      presence.slice(dropIndex).find(event => event.kernel === kernel.kernelId && event.status === "OFFLINE"))
    assert.equal(offline.relayDisconnect, "reply_queue_full")
    const acknowledge = await until("the relay accepted the reconnect", () => releaseAck)
    acknowledge()
    const recovered = await until("kernel reported online again", () =>
      presence.find(event => event.kernel === kernel.kernelId && event.status === "ONLINE" && event.time > offline.time))
    assert.equal(recovered.relayDisconnect, undefined)
    console.log("Reply queue PASS offline reported the queue closure, and reconnect cleared it")
    return
  }
  const oldWireUrl = await relayProxy(relayPort, "refuse")
  const ackUrl = await relayProxy(relayPort, "ack")
  const refused = await startKernel("refused", oldWireUrl, cloudPort)
  const project = join(refused.home, "project")
  await mkdir(project)
  const created = await request(refused.socket, { CreateSession: {
    workspace_id: project, worktree_id: project, alias: null, slice_ref: null, no_subscribe: true,
  } })
  const session = created.SessionCreated.session.id
  const attached = await request(refused.socket, { AttachToSession: {
    session_id: session, client_id: "relay-registration-drill", capability_level: "FullTerminal",
  } })
  const attachment = attached.SessionAttached.attachment.id
  const send = await until("first registration from the refused kernel", () => releaseRegistration)
  if (baseline) {
    await until("premature online presence", () => online(refused))
    console.log("RG4 RED kernel reported online while registration was held before the relay")
    send()
    await until("old relay to drop the registration", () => refusedSocketsClosed > 0)
    const refusedStatus = await status(refused)
    assert.equal(refusedStatus.connected, false)
    assert.equal(forwardedRefusals, 0)
    assert.equal(refusedStatus.registration_refused, undefined)
    assert.equal((await notices(refused, session, attachment)).length, 0)
    console.log("RG0 RED no upgrade reason")
    console.log("RG1 RED socket closed without a reason frame")
    console.log("RG2 RED disconnected status and no owner notice")
    console.log("RG3 RED no registration retry schedule in status")
    const healthyBaseline = await startKernel("baseline-healthy", `ws://127.0.0.1:${relayPort}`, cloudPort)
    await until("unmodified L12 kernel online", () => online(healthyBaseline))
    await until("unmodified L12 kernel connected in status", async () => (await status(healthyBaseline)).connected)
    return
  }
  assert.equal((await status(refused)).connected, false)
  send()
  const first = await until("actionable refusal", async () => (await status(refused)).registration_refused)
  // The relay names an older kernel by "update needed" since L12.1 (relay peer 450).
  assert.match(first.reason, /^update needed: this kernel runs protocol 1011 and is older than the relay/)
  assert(first.next_step)
  assert.equal((await status(refused)).connected, false)
  console.log("RG0 PASS", first.reason)
  const delivered = await notices(refused, session, attachment)
  assert.equal(delivered.length, 1)
  assert.match(delivered[0].message, /Next step: /)
  console.log("RG2 PASS owner received the refusal and next step")

  // Send a current registration missing its required proof to the same relay.
  const malformed = await open(`ws://127.0.0.1:${relayPort}`)
  const closeMessage = new Promise(resolve => malformed.once("message", bytes => resolve(JSON.parse(bytes.toString()))))
  malformed.send(JSON.stringify({ kind: "daemon_register", registration: { local_protocol: 1012 } }))
  const reasonFrame = await Promise.race([closeMessage, delay(2000).then(() => null)])
  assert.deepEqual(reasonFrame, {
    kind: "close", reason: "invalid relay registration: missing registration proof",
  })
  console.log("RG1 PASS malformed registration received a reason frame")

  const healthy = await startKernel("healthy", ackUrl, cloudPort)
  const ack = await until("relay acceptance waiting to be delivered", () => releaseAck)
  assert.equal((await status(healthy)).connected, false)
  assert.equal(online(healthy), false)
  ack()
  await until("online presence after acknowledgment", () => online(healthy))
  assert.equal((await status(healthy)).connected, true)
  assert.equal(online(refused), false)
  console.log("RG4 PASS only the acknowledged kernel published online")

  assert(first.next_retry_at_ms - first.since_ms >= 15_000)
  const second = await until("second refusal and scheduled retry", async () => {
    const current = (await status(refused)).registration_refused
    return current?.next_retry_at_ms > first.next_retry_at_ms && current
  }, 70_000)
  assert.equal(second.since_ms, first.since_ms)
  assert.equal(attempts.length, 2)
  assert(attempts[1] - attempts[0] >= 14_000)
  assert(second.next_retry_at_ms - attempts[1] >= 30_000)
  assert.equal((await notices(refused, session, attachment)).length, 0)
  assert.equal(online(refused), false)
  console.log("RG3 PASS", JSON.stringify({ first: first.next_retry_at_ms, second: second.next_retry_at_ms }))

  execFileSync("systemctl", ["stop", healthy.unit])
  const compatible = await startKernel("l12", `ws://127.0.0.1:${relayPort}`, cloudPort, oldKernelBinary)
  await until("L12 kernel on the new relay", () => online(compatible))
  const olderProtocol = (await status(compatible)).local_daemon_protocol_version
  assert(olderProtocol >= 1012 && olderProtocol < 1014, `older kernel protocol ${olderProtocol}`)
  await delay(500)
  assert.equal((await status(compatible)).connected, true)
  console.log("L12 rollout PASS protocol", olderProtocol, "remains connected on the new relay")

  execFileSync("systemctl", ["stop", compatible.unit])
  const oldRelayPort = await startRelay("l12-relay", oldRelayBinary)
  const olderRelay = await startKernel("older-relay", `ws://127.0.0.1:${oldRelayPort}`, cloudPort)
  const timeout = await until("actionable older relay timeout", async () =>
    (await status(olderRelay)).registration_refused)
  assert.equal(timeout.reason, "relay did not acknowledge registration")
  assert.match(timeout.next_step, /update the relay/)
  assert.equal(online(olderRelay), false)
  console.log("Older relay PASS acknowledgment deadline left actionable status")
}

try {
  await drill()
} finally {
  for (const unit of units.reverse()) {
    spawnSync("systemctl", ["stop", unit], { stdio: "ignore" })
  }
  for (const socket of sockets) socket.terminate()
  for (const server of servers) {
    server.closeAllConnections()
    server.close()
  }
  await rm(scratch, { recursive: true, force: true })
}

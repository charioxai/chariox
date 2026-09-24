#!/usr/bin/env node

import { createHash, createHmac } from "node:crypto"
import { spawn } from "node:child_process"
import { access, chmod, mkdir, writeFile } from "node:fs/promises"
import { constants as fsConstants } from "node:fs"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import { fileURLToPath } from "node:url"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { bounded } from "./lib/local-rust-fault-drill-runtime.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const issuer = "chariox-cloud-drill"
const signingSecret = "relay-identity-drill-secret"
const defaultCargoTarget = path.join(os.homedir(), ".chariox", "dev", "browser-computer-use", "cargo-target")
const defaultRelayBinary = process.env.CHARIOX_RELAY_IDENTITY_BINARY
  ?? path.join(process.env.CARGO_TARGET_DIR ?? defaultCargoTarget, "debug", "chariox-relay")
const CASE_IDS = Object.freeze([
  "auth.accepted-token-expires",
  "auth.expired-token-rejected",
  "auth.clock-skew-tolerance",
  "auth.future-issued-token-rejected",
  "auth.jwt-format",
  "auth.identity-binding-rejected",
  "isolation.cross-realm",
  "continuity.healthy-routed-round-trip",
  "cleanup.resources",
])

let WebSocketImpl = null

function parseArgs(argv) {
  const options = {
    keepArtifactsOnFailure: false,
    relayBinary: defaultRelayBinary,
    reportPath: null,
    dryRun: false,
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--relay-binary") options.relayBinary = readValue(argv, index++, arg)
    else if (arg.startsWith("--relay-binary=")) options.relayBinary = arg.slice("--relay-binary=".length)
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--help" || arg === "-h") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  options.relayBinary = externalPath(options.relayBinary, "relay binary")
  options.reportPath = externalPath(options.reportPath ?? defaultReportPath(), "evidence report")
  if (options.help) {
    console.log([
      "Usage: node apps/cli/scripts/live-relay-identity-security-drill.mjs [options]",
      "",
      "  --relay-binary PATH        Exact external chariox-relay binary",
      "  --report PATH              External JSON evidence report",
      "  --dry-run                  Record the contract without starting the relay",
      "  --keep-artifacts-on-failure",
      "  --help",
    ].join("\n"))
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function externalPath(value, label) {
  if (!path.isAbsolute(value)) throw new Error(`${label} must be absolute`)
  const normalized = path.normalize(value)
  const relative = path.relative(repoRoot, normalized)
  const withinRepo = relative === ""
    || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))
  if (withinRepo) throw new Error(`${label} must stay outside repositories`)
  return normalized
}

function defaultReportPath(now = new Date()) {
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(
    os.homedir(),
    ".codex",
    "evidence",
    "browser-computer-use",
    "relay-token-expiry",
    stamp,
    "report.json",
  )
}

function base64url(input) {
  return Buffer.from(input).toString("base64url")
}

function signToken(tokenClaims) {
  const header = base64url(JSON.stringify({ alg: "HS256", typ: "JWT" }))
  const payload = base64url(JSON.stringify({
    iss: tokenClaims.issuer,
    sub: tokenClaims.subject,
    subject_kind: tokenClaims.subject_kind,
    realm_id: tokenClaims.realm_id,
    allowed_actions: tokenClaims.allowed_actions.map(jwtActionName),
    allowed_targets: tokenClaims.allowed_targets,
    iat: Math.floor(tokenClaims.issued_at_ms / 1_000),
    exp: Math.ceil(tokenClaims.expires_at_ms / 1_000),
    jti: tokenClaims.token_id,
    account_id: tokenClaims.account_id,
    organization_id: tokenClaims.organization_id,
    user_id: null,
    device_id: tokenClaims.device_id,
    machine_id: tokenClaims.machine_id,
    client_id: tokenClaims.client_id,
    public_key_thumbprint: tokenClaims.public_key_thumbprint,
    entitlements_version: tokenClaims.entitlements_version,
  }))
  const signingInput = `${header}.${payload}`
  const signature = createHmac("sha256", signingSecret).update(signingInput).digest("base64url")
  return `${signingInput}.${signature}`
}

function jwtActionName(action) {
  const names = {
    daemon_register: "daemon.register",
    daemon_heartbeat: "daemon.heartbeat",
    client_metadata_read: "client.metadata.read",
    client_connect: "client.connect",
    packet_route: "packet.route",
    peer_request: "peer.request",
    peer_event: "peer.event",
  }
  const name = names[action]
  if (!name) throw new Error(`unsupported relay action ${action}`)
  return name
}

function claims({
  subject,
  subjectKind,
  realm,
  actions,
  targets = null,
  issuedAt = Date.now(),
  expiresAt = Date.now() + 60_000,
  publicKeyThumbprint = null,
}) {
  return {
    issuer,
    subject,
    subject_kind: subjectKind,
    realm_id: realm,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: issuedAt,
    expires_at_ms: expiresAt,
    token_id: `${subject}-${issuedAt}`,
    account_id: "account-drill",
    organization_id: null,
    device_id: subject,
    machine_id: subjectKind === "kernel" || subjectKind === "machine" ? subject : null,
    client_id: subjectKind === "client" ? subject : null,
    public_key_thumbprint: publicKeyThumbprint,
    entitlements_version: "drill",
  }
}

function daemonRegistration({ token, daemonId, machineId }) {
  return {
    kind: "daemon_register",
    registration: {
      auth_token: token,
      daemon_id: daemonId,
      machine_id: machineId,
      machine_alias: machineId,
      os_name: "drill-os",
      kernel_started_at_ms: Date.now(),
      daemon_alias: daemonId,
      kernel_alias: daemonId,
      public_key: `${daemonId}-public-key`,
      capabilities: ["kernel_ws"],
      available_providers: ["codex"],
      accepting_remote_leases: false,
      leased_agent_count: 0,
      local_session_count: 0,
    },
  }
}

function relayPublicKeyThumbprint(publicKey) {
  return createHash("sha256").update(publicKey).digest("hex")
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port
      server.close(() => resolve(port))
    })
    server.on("error", reject)
  })
}

async function connect(url, sockets) {
  return await new Promise((resolve, reject) => {
    const socket = new WebSocketImpl(url)
    const onError = (error) => reject(error)
    socket.once("error", onError)
    socket.once("open", () => {
      socket.off("error", onError)
      sockets.add(socket)
      socket.once("close", () => sockets.delete(socket))
      resolve(socket)
    })
  })
}

async function waitForRelay(url, sockets, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const socket = await connect(url, sockets)
      socket.close()
      return
    } catch {
      await sleep(100)
    }
  }
  throw new Error(`relay did not accept websocket connections at ${url}`)
}

async function sendJson(socket, payload) {
  await new Promise((resolve, reject) => {
    socket.send(JSON.stringify(payload), (error) => error ? reject(error) : resolve())
  })
}

async function nextJson(socket, label = "relay message", timeoutMs = 3_000) {
  return await new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer)
      socket.off("message", onMessage)
      socket.off("close", onClose)
      socket.off("error", onError)
    }
    const onMessage = (data) => {
      cleanup()
      try {
        resolve(JSON.parse(String(data)))
      } catch (error) {
        reject(error)
      }
    }
    const onClose = () => {
      cleanup()
      reject(new Error(`relay connection closed while waiting for ${label}`))
    }
    const onError = (error) => {
      cleanup()
      reject(error)
    }
    const timer = setTimeout(() => {
      cleanup()
      reject(new Error(`timed out waiting for ${label}`))
    }, timeoutMs)
    socket.once("message", onMessage)
    socket.once("close", onClose)
    socket.once("error", onError)
  })
}

async function expectRejected(url, sockets, payload, label) {
  const socket = await connect(url, sockets)
  try {
    const outcome = waitForRejection(socket, label)
    await sendJson(socket, payload)
    await outcome
  } finally {
    socket.close()
  }
}

async function waitForRejection(socket, label, timeoutMs = 3_000) {
  return await new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timer)
      socket.off("message", onMessage)
      socket.off("close", onClose)
      socket.off("error", onError)
    }
    const onMessage = (data) => {
      let envelope
      try {
        envelope = JSON.parse(String(data))
      } catch {
        return
      }
      if (envelope.kind === "close") {
        cleanup()
        resolve(envelope.reason ?? null)
      } else {
        cleanup()
        reject(new Error(`${label} unexpectedly received ${String(data)}`))
      }
    }
    const onClose = () => {
      cleanup()
      resolve(null)
    }
    const onError = () => {
      cleanup()
      resolve(null)
    }
    const timer = setTimeout(() => {
      cleanup()
      reject(new Error(`${label} was not rejected`))
    }, timeoutMs)
    socket.on("message", onMessage)
    socket.once("close", onClose)
    socket.once("error", onError)
  })
}

async function waitForTokenExpiry(socket, acceptedAt) {
  const reason = await waitForRejection(socket, "accepted expiring token", 6_000)
  if (reason !== "relay token expired") {
    throw new Error(`accepted expiring token closed without the expiry reason: ${reason}`)
  }
  return Date.now() - acceptedAt
}

async function requestMetadata(socket, authToken, requestId) {
  const response = nextJson(socket, requestId)
  await sendJson(socket, {
    kind: "client_metadata_request",
    request_id: requestId,
    auth_token: authToken,
    query: { kind: "list_live_machines" },
  })
  return await response
}

function encryptedPayload(marker) {
  return {
    sender_public_key: "relay-token-expiry-drill",
    nonce: `nonce-${marker}`,
    ciphertext: marker,
  }
}

async function routedRoundTrip(client, daemon, requestId) {
  const requestMarker = `${requestId}-request`
  const responseMarker = `${requestId}-response`
  const daemonRequest = nextJson(daemon, `${requestId} daemon request`)
  const clientResponse = nextJson(client, `${requestId} client response`)
  await sendJson(client, {
    kind: "client_request",
    request_id: requestId,
    target: { daemon_id: "daemon-a", daemon_alias: null },
    encrypted_request: encryptedPayload(requestMarker),
  })
  const routedRequest = await daemonRequest
  requireCondition(routedRequest.kind === "daemon_request", "healthy daemon did not receive routed client request", routedRequest)
  requireCondition(routedRequest.encrypted_request?.ciphertext === requestMarker, "healthy routed request payload changed", routedRequest)
  await sendJson(daemon, {
    kind: "daemon_response",
    relay_request_id: routedRequest.relay_request_id,
    encrypted_response: encryptedPayload(responseMarker),
    error: null,
  })
  const routedResponse = await clientResponse
  requireCondition(routedResponse.kind === "client_response", "healthy client did not receive routed daemon response", routedResponse)
  requireCondition(routedResponse.request_id === requestId, "healthy routed response used the wrong request ID", routedResponse)
  requireCondition(routedResponse.error == null, "healthy routed response returned an error", routedResponse)
  requireCondition(routedResponse.encrypted_response?.ciphertext === responseMarker, "healthy routed response payload changed", routedResponse)
}

function requireCondition(condition, message, detail = null) {
  if (!condition) {
    const suffix = detail == null ? "" : `\n${JSON.stringify(detail, null, 2)}`
    throw new Error(`${message}${suffix}`)
  }
}

async function resourceSnapshot(label, relayPid = null) {
  const [memoryPressure, disk, childProcess] = await Promise.all([
    runDiagnostic("memory_pressure", ["-Q"]),
    runDiagnostic("df", ["-k", "/System/Volumes/Data"]),
    relayPid ? runDiagnostic("ps", ["-p", String(relayPid), "-o", "pid=,rss=,%cpu=,etime="]) : null,
  ])
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    memoryPressure: memoryPressure?.stdout.trim() || null,
    disk: disk?.stdout.trim().split("\n").at(-1) || null,
    childProcess: childProcess?.stdout.trim() || null,
  }
}

async function runDiagnostic(command, args) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    const stdout = []
    const stderr = []
    child.stdout.on("data", (chunk) => stdout.push(chunk))
    child.stderr.on("data", (chunk) => stderr.push(chunk))
    child.once("error", () => resolve(null))
    child.once("close", (code) => resolve({
      code,
      stdout: Buffer.concat(stdout).toString("utf8"),
      stderr: Buffer.concat(stderr).toString("utf8"),
    }))
  })
}

async function writeReport(reportPath, report) {
  await mkdir(path.dirname(reportPath), { recursive: true, mode: 0o700 })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 })
  await chmod(reportPath, 0o600)
}

function terminateGroup(child, signal) {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return
  try {
    process.kill(-child.pid, signal)
  } catch {
    child.kill(signal)
  }
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  terminateGroup(child, "SIGTERM")
  const exited = await Promise.race([
    new Promise((resolve) => child.once("close", () => resolve(true))),
    sleep(1_000).then(() => false),
  ])
  if (!exited) {
    terminateGroup(child, "SIGKILL")
    await new Promise((resolve) => child.once("close", resolve))
  }
}

async function closeSockets(sockets) {
  for (const socket of sockets) socket.close()
  await sleep(50)
  for (const socket of sockets) socket.terminate()
  sockets.clear()
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) return
  const sourceCommit = (await runDiagnostic("git", ["rev-parse", "HEAD"]))?.stdout.trim() ?? null
  const report = {
    schema: "chariox.relay_identity_security_drill.v1",
    startedAt: new Date().toISOString(),
    status: options.dryRun ? "dry-run" : "running",
    caseIds: CASE_IDS,
    source: { commit: sourceCommit },
    command: { name: options.relayBinary, args: [] },
    evidenceRoot: path.dirname(options.reportPath),
    resources: [],
    cleanup: null,
  }
  if (options.dryRun) {
    report.completedAt = new Date().toISOString()
    await writeReport(options.reportPath, report)
    console.log(JSON.stringify({ status: report.status, reportPath: options.reportPath }))
    return
  }

  await access(options.relayBinary, fsConstants.X_OK)
  const rootDir = path.join(os.tmpdir(), `chariox-relay-identity-security-${process.pid}-${Date.now()}`)
  const sockets = new Set()
  let relay = null
  let relayStderr = ""
  let interrupted = null
  let failure = null
  let port = null
  let url = null
  const passedChecks = []
  const signalHandlers = new Map(
    ["SIGINT", "SIGTERM"].map((signal) => [signal, () => {
      interrupted ??= signal
      terminateGroup(relay, "SIGTERM")
    }]),
  )
  for (const [signal, handler] of signalHandlers) process.once(signal, handler)
  await prepareDrillArtifacts(rootDir)

  try {
    const websocketModule = await import("ws")
    WebSocketImpl = websocketModule.default
    report.resources.push(await resourceSnapshot("before"))
    port = await freePort()
    url = `ws://127.0.0.1:${port}`
    relay = spawn(options.relayBinary, [], {
      cwd: repoRoot,
      detached: process.platform !== "win32",
      env: {
        ...process.env,
        CHARIOX_RELAY_HOST: "127.0.0.1",
        CHARIOX_RELAY_PORT: String(port),
        CHARIOX_RELAY_SCOPED_ISSUER: issuer,
        CHARIOX_RELAY_SCOPED_HMAC_SECRET: signingSecret,
      },
      stdio: ["ignore", "ignore", "pipe"],
    })
    relay.stderr.on("data", (chunk) => { relayStderr += String(chunk) })
    await waitForRelay(url, sockets)
    report.resources.push(await resourceSnapshot("during", relay.pid))
    if (interrupted) throw new Error(`relay identity security drill interrupted by ${interrupted}`)

    const daemonAToken = signToken(claims({
      subject: "daemon-a",
      subjectKind: "kernel",
      realm: "realm-a",
      actions: ["daemon_register", "daemon_heartbeat", "peer_request", "peer_event"],
      publicKeyThumbprint: relayPublicKeyThumbprint("daemon-a-public-key"),
    }))
    const daemonBToken = signToken(claims({
      subject: "daemon-b",
      subjectKind: "kernel",
      realm: "realm-b",
      actions: ["daemon_register"],
      publicKeyThumbprint: relayPublicKeyThumbprint("daemon-b-public-key"),
    }))
    const clientAToken = signToken(claims({
      subject: "client-a",
      subjectKind: "client",
      realm: "realm-a",
      actions: ["client_connect", "client_metadata_read", "packet_route"],
    }))
    const clientBToken = signToken(claims({
      subject: "client-b",
      subjectKind: "client",
      realm: "realm-b",
      actions: ["client_connect", "client_metadata_read"],
    }))

    const daemonA = await connect(url, sockets)
    await sendJson(daemonA, daemonRegistration({ token: daemonAToken, daemonId: "daemon-a", machineId: "machine-a" }))
    const daemonB = await connect(url, sockets)
    await sendJson(daemonB, daemonRegistration({ token: daemonBToken, daemonId: "daemon-b", machineId: "machine-b" }))
    await sleep(100)
    requireCondition(daemonA.readyState === WebSocketImpl.OPEN, "daemon A registration was rejected")
    requireCondition(daemonB.readyState === WebSocketImpl.OPEN, "daemon B registration was rejected")

    const clientA = await connect(url, sockets)
    const connectedAResponse = nextJson(clientA, "client A connect")
    await sendJson(clientA, {
      kind: "client_connect",
      auth_token: clientAToken,
      target: { daemon_id: "daemon-a", daemon_alias: null },
    })
    requireCondition((await connectedAResponse).kind === "client_connected", "valid realm A client did not connect")

    const metadataA = await connect(url, sockets)
    const initialMetadataA = await requestMetadata(metadataA, clientAToken, "metadata-a")
    requireCondition(initialMetadataA.machines?.length === 1, "realm A metadata leaked or missed machines", initialMetadataA)
    requireCondition(initialMetadataA.machines[0].machine_id === "machine-a", "realm A metadata returned the wrong machine", initialMetadataA)

    const expiringIssuedAt = Date.now()
    const expiringToken = signToken(claims({
      subject: "client-expiring",
      subjectKind: "client",
      realm: "realm-a",
      actions: ["client_connect"],
      targets: ["daemon-a"],
      issuedAt: expiringIssuedAt,
      expiresAt: expiringIssuedAt + 3_000,
    }))
    const expiringClient = await connect(url, sockets)
    const expiringConnected = nextJson(expiringClient, "expiring client connect")
    await sendJson(expiringClient, {
      kind: "client_connect",
      auth_token: expiringToken,
      target: { daemon_id: "daemon-a", daemon_alias: null },
    })
    requireCondition((await expiringConnected).kind === "client_connected", "short-lived client token was not accepted")
    const acceptedAt = Date.now()
    const expiry = waitForTokenExpiry(expiringClient, acceptedAt)
    const healthyDuringExpiry = requestMetadata(metadataA, clientAToken, "metadata-during-expiry")
    const routedDuringExpiry = routedRoundTrip(clientA, daemonA, "route-during-expiry")
    const [expiryLatencyMs, metadataDuringExpiry] = await Promise.all([expiry, healthyDuringExpiry, routedDuringExpiry])
    requireCondition(metadataDuringExpiry.machines?.[0]?.machine_id === "machine-a", "healthy metadata peer stalled during another token expiry", metadataDuringExpiry)
    await routedRoundTrip(clientA, daemonA, "route-after-expiry")
    report.timings = { acceptedTokenExpiryLatencyMs: expiryLatencyMs }
    passedChecks.push("accepted short-lived token expired while healthy metadata and routed request peers remained live")

    const expiredToken = signToken(claims({
      subject: "client-expired",
      subjectKind: "client",
      realm: "realm-a",
      actions: ["client_connect"],
      targets: ["daemon-a"],
      expiresAt: Date.now() - 2_000,
    }))
    await expectRejected(url, sockets, {
      kind: "client_connect",
      auth_token: expiredToken,
      target: { daemon_id: "daemon-a", daemon_alias: null },
    }, "already-expired client token")
    passedChecks.push("already-expired token rejected")

    const skewToken = signToken(claims({
      subject: "client-skew-accepted",
      subjectKind: "client",
      realm: "realm-a",
      actions: ["client_metadata_read"],
      issuedAt: Date.now() + 30_000,
      expiresAt: Date.now() + 90_000,
    }))
    const skewClient = await connect(url, sockets)
    const skewMetadata = await requestMetadata(skewClient, skewToken, "metadata-skew-accepted")
    requireCondition(skewMetadata.machines?.[0]?.machine_id === "machine-a", "token within clock-skew tolerance was rejected", skewMetadata)
    passedChecks.push("token issued 30 seconds ahead accepted")

    const futureToken = signToken(claims({
      subject: "client-future-rejected",
      subjectKind: "client",
      realm: "realm-a",
      actions: ["client_connect"],
      targets: ["daemon-a"],
      issuedAt: Date.now() + 62_000,
      expiresAt: Date.now() + 120_000,
    }))
    await expectRejected(url, sockets, {
      kind: "client_connect",
      auth_token: futureToken,
      target: { daemon_id: "daemon-a", daemon_alias: null },
    }, "future-issued client token")
    passedChecks.push("token issued beyond clock-skew tolerance rejected")

    await expectRejected(url, sockets, {
      kind: "client_connect",
      auth_token: "invalid-client-token",
      target: { daemon_id: "daemon-a", daemon_alias: null },
    }, "invalid client token")
    await expectRejected(url, sockets, daemonRegistration({
      token: clientAToken,
      daemonId: "daemon-client-token",
      machineId: "machine-client-token",
    }), "client token daemon registration")
    await expectRejected(url, sockets, {
      kind: "client_metadata_request",
      request_id: "daemon-token-metadata",
      auth_token: daemonAToken,
      query: { kind: "list_live_machines" },
    }, "daemon token metadata query")
    const mismatchedKeyToken = signToken(claims({
      subject: "daemon-key-mismatch",
      subjectKind: "kernel",
      realm: "realm-a",
      actions: ["daemon_register"],
      publicKeyThumbprint: relayPublicKeyThumbprint("another-public-key"),
    }))
    await expectRejected(url, sockets, daemonRegistration({
      token: mismatchedKeyToken,
      daemonId: "daemon-key-mismatch",
      machineId: "machine-key-mismatch",
    }), "daemon key binding mismatch")
    passedChecks.push("invalid, role-mismatched, and key-mismatched identities rejected")

    await expectRejected(url, sockets, {
      kind: "client_connect",
      auth_token: clientAToken,
      target: { daemon_id: "daemon-b", daemon_alias: null },
    }, "cross-realm client route")
    const metadataB = await connect(url, sockets)
    const realmBMetadata = await requestMetadata(metadataB, clientBToken, "metadata-b")
    requireCondition(realmBMetadata.machines?.length === 1, "realm B metadata leaked or missed machines", realmBMetadata)
    requireCondition(realmBMetadata.machines[0].machine_id === "machine-b", "realm B metadata returned the wrong machine", realmBMetadata)
    passedChecks.push("cross-realm route rejected and both realm inventories stayed isolated")

    report.probe = {
      acceptedTokenExpired: true,
      expiredTokenRejected: true,
      clockSkewAccepted: true,
      futureIssuedTokenRejected: true,
      jwtFormatAccepted: true,
      identityBindingRejected: true,
      crossRealmRejected: true,
      healthyRoutedRoundTrip: true,
    }
    report.status = "passed"
  } catch (error) {
    failure = error
    report.status = "failed"
    report.failure = bounded(error instanceof Error ? error.message : error)
  } finally {
    await closeSockets(sockets)
    await stopChild(relay)
    const ownedProcessesAbsent = relay == null || relay.exitCode !== null || relay.signalCode !== null
    report.cleanup = {
      ownedProcessesAbsent,
      relayExitCode: relay?.exitCode ?? null,
      relayExitSignal: relay?.signalCode ?? null,
    }
    report.resources.push(await resourceSnapshot("after-cleanup"))
    report.completedAt = new Date().toISOString()
    report.passedChecks = passedChecks
    if (relayStderr.trim()) report.output = { relayStderrTail: bounded(relayStderr) }
    if (!ownedProcessesAbsent && !failure) {
      failure = new Error("relay identity security drill left its relay process running")
      report.status = "failed"
      report.failure = failure.message
    }
    await writeReport(options.reportPath, report)
    await finalizeDrillArtifacts({
      rootDir,
      passed: report.status === "passed",
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: "relay-identity-security",
        url,
        port,
        issuer,
        passedChecks,
        relayStderrTail: bounded(relayStderr),
      },
      log: (name, details) => console.log(`[relay-identity-security-drill] ${name}`, JSON.stringify(details)),
    })
    for (const [signal, handler] of signalHandlers) process.removeListener(signal, handler)
  }
  console.log(JSON.stringify({ status: report.status, reportPath: options.reportPath }))
  if (failure) throw failure
}

main().catch((error) => {
  console.error(`[relay-identity-security-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})

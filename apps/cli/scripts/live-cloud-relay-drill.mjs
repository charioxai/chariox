#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm, stat } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cloudRoot = process.env.ARROBA_CLOUD_REPO
  ? path.resolve(process.env.ARROBA_CLOUD_REPO)
  : path.resolve(repoRoot, "..", "arroba-cloud")
const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgresql://arroba:arroba@localhost:5432/arroba_cloud"
const CLOUD_SECRET = "arroba-cloud-live-drill-secret"
const CLOUD_ISSUER = "arroba-cloud-live-drill"

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[cloud-relay-drill] ${name}`)
  else console.log(`[cloud-relay-drill] ${name}`, JSON.stringify(details))
}

function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
  }
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

function spawnProcess(command, args, options) {
  return spawn(command, args, {
    ...options,
    stdio: ["ignore", "pipe", "pipe"],
  })
}

async function waitForHttp(url, timeoutMs = 30_000) {
  const started = Date.now()
  let lastError = null
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url)
      if (response.ok) return
      lastError = new Error(`status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${url}: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (exists) return binary
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

function makePorts() {
  const base = 52000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 2000,
    opencodePort: base + 3000,
    codexPort: base + 3001,
    cloudPort: base + 4000,
  }
}

async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const created = unwrap(
        await probe.send(requests.createSessionRequest(workspace, workspace)),
        "SessionCreated",
      )
      await probe.send(requests.endSessionRequest(created.session.id)).catch(() => {})
      await probe.close()
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`local daemon did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(requests.listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error("probe timeout") }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

async function main() {
  const ports = makePorts()
  const runId = `cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), runId)
  const workspace = path.join(rootDir, "workspace")
  const daemonId = `cloud-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `cloud-home-${process.pid}`
  const clientId = `cloud-cli-${process.pid}-${Date.now()}`
  const apiUrl = `http://127.0.0.1:${ports.cloudPort}`
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })

  const [{ LocalIpcClient }, requests, cloudRelay, cloudDb] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../dist/cloud-relay.js"),
    import(path.join(cloudRoot, "packages/db/dist/index.js")),
  ])

  const kernelPath = await buildKernelIfNeeded()
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: CLOUD_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: CLOUD_SECRET,
  }
  const daemonEnv = {
    ...process.env,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "session-history"),
  }
  const cloudEnv = {
    ...process.env,
    HOST: "127.0.0.1",
    PORT: String(ports.cloudPort),
    DATABASE_URL,
    ARROBA_CLOUD_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_CLOUD_ISSUER_ID: CLOUD_ISSUER,
    ARROBA_CLOUD_RELAY_TOKEN_SECRET: CLOUD_SECRET,
  }

  let relay = null
  let daemon = null
  let cloudServer = null
  let localClient = null
  let remoteClient = null
  const db = cloudDb.createCloudDatabase({ databaseUrl: DATABASE_URL })

  try {
    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`arroba cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-cloud")
    const cloudBuild = await run("pnpm", ["run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudBuild.code !== 0) {
      throw new Error(`arroba-cloud build failed\n${cloudBuild.stdout}\n${cloudBuild.stderr}`)
    }
    const migrate = await run("pnpm", ["--filter", "@arroba-cloud/db", "run", "prisma:migrate"], {
      cwd: cloudRoot,
      env: cloudEnv,
    })
    if (migrate.code !== 0) {
      throw new Error(`arroba-cloud migrate failed\n${migrate.stdout}\n${migrate.stderr}`)
    }

    log("start-cloud")
    cloudServer = spawnProcess("node", [path.join(cloudRoot, "apps/api/dist/server.js")], {
      cwd: cloudRoot,
      env: cloudEnv,
    })
    await waitForHttp(`${apiUrl}/health`)

    log("start-relay-and-kernel")
    relay = spawnProcess("cargo", ["run", "--manifest-path", path.join(repoRoot, "apps/relay/Cargo.toml"), "--bin", "arroba-relay"], {
      cwd: repoRoot,
      env: relayEnv,
    })
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace)
    localClient = new LocalIpcClient(kernelUrl)

    log("bootstrap-cloud-profile")
    let profile = await cloudRelay.bootstrapCloudRelayProfile({
      apiUrl,
      email: `${runId}@example.com`,
      accountSlug: runId,
    })
    profile = await cloudRelay.pairCloudRelayClient(profile, clientId, "drill-cli")

    log("mint-kernel-token")
    const daemonRelay = await cloudRelay.issueCloudRelayToken({
      profile,
      subject: daemonId,
      subjectKind: "kernel",
      userId: profile.userId,
    })
    await localClient.send(
      requests.configureRelayRequest(daemonRelay.relayUrl, daemonRelay.relayToken),
    )

    log("mint-client-token")
    const clientRelay = await cloudRelay.issueCloudRelayToken({
      profile,
      subject: profile.clientId ?? clientId,
      subjectKind: "client",
      userId: profile.userId,
      clientId: profile.clientId ?? clientId,
      allowedTargets: [daemonAlias],
    })

    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      clientRelay.relayUrl,
      clientRelay.relayToken,
      daemonAlias,
    )
    remoteClient = new LocalIpcClient(clientRelay.relayUrl, {
      relayAuthToken: clientRelay.relayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    log("remote-session-create")
    const created = unwrap(
      await remoteClient.send(requests.createSessionRequest(workspace, workspace)),
      "SessionCreated",
    )
    assert(created.session?.id, "remote cloud session creation should return a session", created)

    const attached = unwrap(
      await remoteClient.send(requests.attachToSessionRequest(created.session.id, `${clientId}-remote`)),
      "SessionAttached",
    )
    assert(attached.attachment?.session_id === created.session.id, "remote cloud attach should bind to the created session", attached)

    const listed = unwrap(
      await remoteClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    console.log("live cloud relay drill passed")
  } finally {
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await terminateChild(daemon)
    await terminateChild(relay)
    await terminateChild(cloudServer)
    await db.account.deleteMany({ where: { slug: runId } }).catch(() => {})
    await db.user.deleteMany({ where: { email: `${runId}@example.com` } }).catch(() => {})
    await db.$disconnect().catch(() => {})
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()

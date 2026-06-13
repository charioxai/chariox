#!/usr/bin/env node
import { spawn } from "node:child_process"
import net from "node:net"
import { mkdir, rm, stat, writeFile } from "node:fs/promises"
import path from "node:path"
import os from "node:os"
import { fileURLToPath } from "node:url"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEFAULT_TIMEOUT_MS = 120_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    remote: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++i])
    else if (arg === "--poll-ms") options.pollMs = Number(argv[++i])
    else if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--remote") options.remote = true
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-vault-unlock-tui-drill.mjs [--remote] [--timeout-ms 120000]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[vault-tui-drill] ${name}`)
  else console.log(`[vault-tui-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function makePorts() {
  const kernelPort = 51000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
    relayPort: kernelPort + 3000,
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

async function ensureBuilt() {
  const cliDist = path.join(repoRoot, "apps/cli/dist/index.js")
  const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const relayBinary = path.join(repoRoot, "apps/relay/target/debug/arroba-relay")
  const cliReady = await stat(cliDist).then((info) => info.isFile()).catch(() => false)
  const kernelReady = await stat(kernelBinary).then((info) => info.isFile()).catch(() => false)
  const relayReady = await stat(relayBinary).then((info) => info.isFile()).catch(() => false)
  if (!cliReady) {
    const result = await run("pnpm", ["--filter", "@arroba/cli", "run", "build"])
    if (result.code !== 0) throw new Error(`cli build failed\n${result.stdout}\n${result.stderr}`)
  }
  if (!kernelReady) {
    const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
    if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  if (!relayReady) {
    const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/relay/Cargo.toml"), "--bin", "arroba-relay"])
    if (result.code !== 0) throw new Error(`relay build failed\n${result.stdout}\n${result.stderr}`)
  }
  return { cliDist, kernelBinary, relayBinary }
}

async function waitForKernel(kernelUrl) {
  const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
  const { listSessionsRequest } = await import("../../../packages/kernel-client/dist/ipc-requests.js")
  const deadline = Date.now() + 30_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForSocket(socketPath) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const client = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        client.once("connect", resolve)
        client.once("error", reject)
      })
      client.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForTcpPort(port, host = "127.0.0.1", timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const connected = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once("connect", () => {
        socket.destroy()
        resolve(true)
      })
      socket.once("error", () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (connected) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
}

async function waitForRelayTarget(LocalIpcClient, listSessionsRequest, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError ?? "unknown error"}`)
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  let nextId = 1
  let buffer = ""
  const pending = new Map()
  socket.on("data", (chunk) => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const newline = buffer.indexOf("\n")
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  socket.on("error", (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForInteraction(automation, agentId, titlePrefix, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let snapshot = null
  while (Date.now() < deadline) {
    snapshot = await automation.send("snapshot")
    const interaction = snapshot.interactions?.find((entry) =>
      entry.agentId === agentId && String(entry.title ?? "").startsWith(titlePrefix))
    if (interaction) return { snapshot, interaction }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for interaction ${titlePrefix} for agent ${agentId}${snapshot ? `\n${JSON.stringify(snapshot, null, 2)}` : ""}`)
}

async function bestEffortWithTimeout(promise, timeoutMs) {
  await Promise.race([
    promise,
    sleep(timeoutMs).then(() => undefined),
  ]).catch(() => undefined)
}

function requestResult(promise) {
  return promise.then(
    (response) => ({ ok: true, response }),
    (error) => ({ ok: false, error }),
  )
}

async function unwrapRequest(resultPromise, label) {
  const result = await resultPromise
  if (!result.ok) {
    throw new Error(`${label} failed: ${result.error?.stack ?? result.error?.message ?? result.error}`)
  }
  return result.response
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, "target", "live-vault-unlock-tui-drill", `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, "workspace")
  const home = path.join(rootDir, "home")
  const configHome = path.join(rootDir, "config")
  const automationSocket = path.join(os.tmpdir(), `arroba-vault-tui-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const relayToken = `vault-tui-relay-token-${process.pid}-${Date.now()}`
  const targetDaemonAlias = `vault-tui-home-${process.pid}`
  const passphrase = `vault-tui-passphrase-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `vault-tui-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_ALIAS: targetDaemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "history"),
    ARROBA_TEST_TUI: "1",
    ...(options.remote ? {
      ARROBA_RELAY_URL: relayUrl,
      ARROBA_RELAY_TOKEN: relayToken,
    } : {}),
  }

  let relay = null
  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, "arroba"), { recursive: true })
    await writeFile(path.join(configHome, "arroba", "config.toml"), [
      "[credential_vault]",
      'backend = "arroba_encrypted"',
      `service = "arroba-vault-tui-${process.pid}"`,
      `path = "${path.join(rootDir, "vault", "vault.db").replaceAll("\\", "\\\\")}"`,
      'unlock_policy = "ttl"',
      "default_ttl_minutes = 30",
      "max_ttl_minutes = 240",
      'agent_management = "allow"',
      "",
    ].join("\n"), "utf8")
    const { cliDist, kernelBinary, relayBinary } = await ensureBuilt()

    if (options.remote) {
      relay = spawn(relayBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          ARROBA_RELAY_HOST: "127.0.0.1",
          ARROBA_RELAY_PORT: String(ports.relayPort),
          ARROBA_RELAY_TOKEN: relayToken,
        },
        stdio: ["ignore", "ignore", "inherit"],
      })
      await waitForTcpPort(ports.relayPort)
      log("relay-ready", { relayUrl })
    }
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ["ignore", "ignore", "inherit"] })
    await waitForKernel(kernelUrl)
    log("kernel-ready", { kernelUrl })

    const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
    const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")
    client = new LocalIpcClient(kernelUrl)
    if (options.remote) {
      await waitForRelayTarget(LocalIpcClient, requests.listSessionsRequest, relayUrl, relayToken, targetDaemonAlias)
      log("relay-target-ready", { targetDaemonAlias })
    }
    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, "vault-tui")), "SessionCreated").session
    const sessionId = session.id
    const agentId = session.default_agent_id ?? session.agents?.[0]?.id
    if (!agentId) throw new Error("created session did not expose an agent")

    const connectionArgs = options.remote
      ? ["--relay-url", relayUrl, "--relay-token", relayToken, "--target-daemon-alias", targetDaemonAlias]
      : ["--kernel-url", kernelUrl]
    const cliArgs = [
      "-q",
      "/dev/null",
      "env",
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      "bun",
      cliDist,
      ...connectionArgs,
      "--automation-socket", automationSocket,
      "--session", sessionId,
      "--workspace", workspace,
      "--worktree", workspace,
      "--provider", "codex",
      "--model", "gpt-5.4",
      "--client-id", `vault-tui-drill-cli-${process.pid}`,
    ]
    cli = spawn("script", cliArgs, { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
    await waitForSocket(automationSocket)
    automation = createAutomationClient(automationSocket)
    const firstSnapshot = await automation.send("snapshot")
    if (firstSnapshot.session?.id !== sessionId) {
      throw new Error(`CLI did not attach to session ${sessionId}: ${JSON.stringify(firstSnapshot)}`)
    }
    log("cli-ready", { sessionId, agentId })

    const firstSet = requestResult(client.send(requests.setCredentialSecretRequest("m26-tui-secret", "m26-tui-secret-value", { sessionId, agentId })))
    const firstUnlock = await waitForInteraction(automation, agentId, "Unlock Arroba Vault", options.timeoutMs, options.pollMs)
    if (firstUnlock.interaction.customChoice?.input_kind !== "secret") {
      throw new Error(`unlock interaction was not secret input: ${JSON.stringify(firstUnlock.interaction)}`)
    }
    await automation.send("interaction_custom_reply", { interactionId: firstUnlock.interaction.id, reply: passphrase })
    await automation.send("interaction_move", { delta: 1 })
    await automation.send("interaction_submit")
    await unwrapRequest(firstSet, "first vault secret set")
    const unlocked = unwrap(await client.send(requests.getCredentialVaultStatusRequest()), "CredentialVaultStatus").status
    if (unlocked.unlocked !== true) throw new Error(`vault should be unlocked after TUI response: ${JSON.stringify(unlocked)}`)
    log("unlock-passed", { choices: firstUnlock.interaction.choices?.map((choice) => choice.id) })

    const manage = requestResult(client.send(requests.manageCredentialVaultRequest(sessionId, agentId)))
    const manageInteraction = await waitForInteraction(automation, agentId, "Arroba Vault Unlocked", options.timeoutMs, options.pollMs)
    await automation.send("interaction_move", { delta: 1 })
    await automation.send("interaction_submit")
    await unwrapRequest(manage, "vault manage")
    const extended = unwrap(await client.send(requests.getCredentialVaultStatusRequest()), "CredentialVaultStatus").status
    if (extended.unlocked !== true) throw new Error(`vault should remain unlocked after extend: ${JSON.stringify(extended)}`)
    log("manage-passed", { choices: manageInteraction.interaction.choices?.map((choice) => choice.id) })

    await client.send(requests.lockCredentialVaultRequest())
    const locked = unwrap(await client.send(requests.getCredentialVaultStatusRequest()), "CredentialVaultStatus").status
    if (locked.unlocked !== false) throw new Error(`vault should be locked after lock: ${JSON.stringify(locked)}`)

    const secondSet = requestResult(client.send(requests.setCredentialSecretRequest("m26-tui-secret-2", "m26-tui-secret-value-2", { sessionId, agentId })))
    const secondUnlock = await waitForInteraction(automation, agentId, "Unlock Arroba Vault", options.timeoutMs, options.pollMs)
    await automation.send("interaction_custom_reply", { interactionId: secondUnlock.interaction.id, reply: passphrase })
    await automation.send("interaction_submit")
    await unwrapRequest(secondSet, "second vault secret set")
    log("second-unlock-passed", { choices: secondUnlock.interaction.choices?.map((choice) => choice.id) })

    await writeFile(path.join(rootDir, "manifest.json"), JSON.stringify({
      ok: true,
      sessionId,
      agentId,
      firstUnlockChoices: firstUnlock.interaction.choices?.map((choice) => choice.id) ?? [],
      manageChoices: manageInteraction.interaction.choices?.map((choice) => choice.id) ?? [],
      secondUnlockChoices: secondUnlock.interaction.choices?.map((choice) => choice.id) ?? [],
      unlocked,
      extended,
    }, null, 2), "utf8")
    succeeded = true
    console.log(JSON.stringify({ ok: true, mode: options.remote ? "vault-unlock-remote-tui" : "vault-unlock-tui", rootDir, sessionId, agentId }, null, 2))
  } finally {
    if (automation) {
      await bestEffortWithTimeout(automation.send("exit"), 2_000)
      automation.close()
    }
    if (client) await bestEffortWithTimeout(client.close(), 5_000)
    await terminateChild(cli)
    await terminateChild(daemon)
    await terminateChild(relay)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log("artifacts-retained", { rootDir })
    }
  }
}

main().catch((error) => {
  console.error(`[vault-tui-drill] failed: ${error.stack || error.message}`)
  process.exitCode = 1
})

#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[kernel-identity-drill] ${name}`)
  else console.log(`[kernel-identity-drill] ${name}`, JSON.stringify(details))
}

function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
  }
}

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (const arg of argv) {
    if (arg === "--") continue
    if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-kernel-identity-session-drill.mjs [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
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

async function buildKernel() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel")
  const result = await run("cargo", [
    "build",
    "--manifest-path",
    path.join(repoRoot, "apps/kernel/Cargo.toml"),
    "--bin",
    "chariox-kernel",
  ])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function startKernel(binary, env, name) {
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.stdout.on("data", (chunk) => {
    for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) log(`${name}:stdout`, line)
  })
  child.stderr.on("data", (chunk) => {
    for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) log(`${name}:stderr`, line)
  })
  child.on("exit", (code, signal) => log(`${name}:exit`, { code, signal }))
  return child
}

async function stopKernel(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

async function waitForKernel(LocalIpcClient, requests, kernelUrl) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(requests.listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready at ${kernelUrl}: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function openClient(LocalIpcClient, requests, kernelUrl) {
  await waitForKernel(LocalIpcClient, requests, kernelUrl)
  return new LocalIpcClient(kernelUrl)
}

async function relayStatus(client, requests) {
  return unwrap(await client.send(requests.relayStatusRequest()), "RelayStatus").status
}

async function listSessionIds(client, requests) {
  return unwrap(await client.send(requests.listSessionsRequest()), "SessionsListed").sessions.map((session) => session.id)
}

function makeEnv(rootDir, port) {
  return {
    ...process.env,
    HOME: path.join(rootDir, "home"),
    XDG_CONFIG_HOME: path.join(rootDir, "xdg-config"),
    XDG_STATE_HOME: path.join(rootDir, "xdg-state"),
    CHARIOX_KERNEL_HOST: "127.0.0.1",
    CHARIOX_KERNEL_PORT: String(port),
    CHARIOX_MCP_PORT: String(port + 1000),
    CHARIOX_OPENCODE_PORT: String(port + 2000),
    CHARIOX_CODEX_PORT: String(port + 2001),
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, `daemon-${port}.sock`),
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const runId = `${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), `chariox-kernel-identity-${runId}`)
  const workspace = path.join(rootDir, "workspace")
  const portA = 54000 + Math.floor(Math.random() * 500)
  const portB = portA + 1
  const urlA = `ws://127.0.0.1:${portA}/kernel`
  const urlB = `ws://127.0.0.1:${portB}/kernel`
  const envA = makeEnv(rootDir, portA)
  const envB = makeEnv(rootDir, portB)
  let kernel = null
  let client = null
  let succeeded = false
  let failure = null
  let sessionId = null
  let statusA1 = null
  let statusA2 = null
  let statusB = null

  const [{ LocalIpcClient }, requests] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
  ])

  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    const binary = await buildKernel()

    log("start-kernel-a", { port: portA })
    kernel = startKernel(binary, envA, "kernel-a")
    client = await openClient(LocalIpcClient, requests, urlA)
    statusA1 = await relayStatus(client, requests)
    const created = unwrap(
      await client.send(requests.createSessionRequest(workspace, workspace, "identity-drill-session")),
      "SessionCreated",
    )
    sessionId = created.session.id
    assert(sessionId, "kernel A should create a session", created)
    log("kernel-a-ready", { kernelId: statusA1.daemon_id, machineId: statusA1.machine_id, sessionId })
    await client.close()
    client = null
    await stopKernel(kernel)
    kernel = null

    log("restart-kernel-a", { port: portA })
    kernel = startKernel(binary, envA, "kernel-a-restart")
    client = await openClient(LocalIpcClient, requests, urlA)
    statusA2 = await relayStatus(client, requests)
    assert(statusA2.daemon_id === statusA1.daemon_id, "same host/port should retain kernel id", { first: statusA1, second: statusA2 })
    assert(statusA2.machine_id === statusA1.machine_id, "same machine should retain machine id", { first: statusA1, second: statusA2 })
    assert((await listSessionIds(client, requests)).includes(sessionId), "same kernel id should restore its session")
    await client.close()
    client = null
    await stopKernel(kernel)
    kernel = null

    log("start-kernel-b", { port: portB })
    kernel = startKernel(binary, envB, "kernel-b")
    client = await openClient(LocalIpcClient, requests, urlB)
    statusB = await relayStatus(client, requests)
    assert(statusB.daemon_id !== statusA1.daemon_id, "different host/port should get a distinct kernel id", { first: statusA1, second: statusB })
    assert(statusB.machine_id === statusA1.machine_id, "different kernel on same OS user should share machine id", { first: statusA1, second: statusB })
    assert(!(await listSessionIds(client, requests)).includes(sessionId), "different kernel id should not list kernel A session")
    await client.close()
    client = null
    await stopKernel(kernel)
    kernel = null

    log("delete-kernel-a")
    kernel = startKernel(binary, envA, "kernel-a-delete")
    client = await openClient(LocalIpcClient, requests, urlA)
    assert((await listSessionIds(client, requests)).includes(sessionId), "kernel A session should reappear before delete")
    const deleted = unwrap(await client.send(requests.deleteKernelRequest()), "KernelDeleted")
    assert(deleted.kernel_id === statusA1.daemon_id, "delete should target kernel A identity", deleted)
    assert(deleted.deleted_sessions?.some((session) => session.id === sessionId), "delete should remove kernel A session", deleted)
    assert(!(await listSessionIds(client, requests)).includes(sessionId), "kernel A session should be gone after delete")
    log("pass")
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close().catch(() => {})
    await stopKernel(kernel)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: "kernel-identity-session",
        workspace,
        portA,
        portB,
        urlA,
        urlB,
        sessionId,
        statusA1,
        statusA2,
        statusB,
      },
      log,
    })
    if (!succeeded && options.keepArtifactsOnFailure) log("artifacts-retained", { rootDir })
  }
}

await main()

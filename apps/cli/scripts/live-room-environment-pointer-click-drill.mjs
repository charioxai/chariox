#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createWriteStream } from "node:fs"
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const kernelClientRoot = path.join(repoRoot, "packages", "kernel-client")
const startedAt = new Date().toISOString()
const stamp = startedAt.replace(/[:.]/g, "-")
const runId = `room-pointer-${process.pid}-${stamp}`
const evidenceRoot = path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", "human-input", stamp)
const containerName = `chariox-slice-${runId}`
const homeVolume = `${containerName}-home`
const kernelPort = 51000 + Math.floor(Math.random() * 1000)
const tempRootPromise = mkdtemp(path.join(os.tmpdir(), "chariox-room-pointer-"))
const children = []
const resources = []
let client = null
let fixture = null
let slice = null
let sessionId = null
let requests = null
let failure = null
let result = null

await mkdir(evidenceRoot, { recursive: true })

try {
  await run()
} catch (error) {
  failure = error
} finally {
  await cleanup()
}

if (failure) {
  console.error(failure?.stack ?? String(failure))
  process.exitCode = 1
} else {
  console.log(JSON.stringify({ status: "passed", evidenceRoot }, null, 2))
}

async function run() {
  const tempRoot = await tempRootPromise
  await assertDockerReady()
  resources.push(await resourceSnapshot("before"))
  fixture = await startFixture()
  await seedConfig(tempRoot)

  const kernelBinary = await resolveKernelBinary()
  const log = createWriteStream(path.join(evidenceRoot, "kernel.log"), { flags: "a" })
  const kernel = spawn(kernelBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      CHARIOX_HOME: path.join(tempRoot, "home"),
      CHARIOX_KERNEL_PORT: String(kernelPort),
      CHARIOX_MCP_PORT: String(kernelPort + 1),
      CHARIOX_CODEX_PORT: String(kernelPort + 2),
      CHARIOX_OPENCODE_PORT: String(kernelPort + 3),
      CHARIOX_DAEMON_SOCKET: path.join(tempRoot, "daemon.sock"),
      CHARIOX_DAEMON_ID: `${runId}-home`,
      CHARIOX_DAEMON_ALIAS: `${runId}-home`,
      CHARIOX_SESSION_HISTORY_DIR: path.join(tempRoot, "history"),
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  kernel.stdout.pipe(log)
  kernel.stderr.pipe(log)
  kernel.once("exit", () => log.end())
  children.push(kernel)

  const [{ LocalIpcClient }, importedRequests] = await Promise.all([
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href),
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc-requests.js")).href),
  ])
  requests = importedRequests
  client = await waitFor(async () => {
    const candidate = new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`)
    try {
      await candidate.send(requests.listSlicesRequest())
      return candidate
    } catch (error) {
      candidate.close?.()
      throw error
    }
  }, 60_000, "kernel did not accept local connections")

  const session = unwrap(
    await client.send(requests.createSessionRequest(repoRoot, repoRoot, runId)),
    "SessionCreated",
  ).session
  sessionId = session.id
  slice = unwrap(await client.send(requests.createSliceRequest({
    name: runId,
    backend: "local_docker",
    displayMode: "headed",
    workspaceMount: repoRoot,
    workerKernelRef: `${runId}-worker`,
    base: "clean",
  })), "SliceCreated").slice
  const binding = unwrap(
    await client.send(requests.bindRoomEnvironmentSliceRequest(sessionId, slice.id)),
    "RoomEnvironmentSlice",
  ).binding
  assert.equal(binding.session_id, sessionId)
  assert.equal(binding.slice_id, slice.id)

  await client.send(requests.startSliceRequest(slice.id))
  slice = await waitForSliceRunning(slice.id)
  const limits = await dockerLimits()
  assert.equal(limits.memoryBytes, 2048 * 1024 * 1024)
  assert.equal(limits.memorySwapBytes, limits.memoryBytes)
  assert.equal(limits.nanoCpus, 1_000_000_000)
  await waitForBrowserReady(60_000)
  await sliceScreen(["open-url", `http://host.docker.internal:${fixture.port}/click`])
  await waitForBrowserText("POINTER_CLICK_READY", 30_000, "click fixture did not load")
  await screenshot("before-click")

  const environment = unwrap(await client.send(requests.startRoomEnvironmentRequest(sessionId, {
    css_width: 1280,
    css_height: 800,
    device_scale_factor: 1,
    desktop_pixel_width: 1280,
    desktop_pixel_height: 800,
  })), "RoomEnvironmentUpdated").environment
  assert.equal(environment.lifecycle, "ready")
  const takeover = unwrap(
    await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(sessionId, { kind: "desktop" })),
    "RoomEnvironmentTakeoverUpdated",
  )
  assert.equal(takeover.outcome.state, "granted")
  const desktopOwner = takeover.environment.input_ownership.find(
    (owner) => owner.target.kind === "desktop",
  )
  assert.equal(desktopOwner?.actor_id, "user:local")

  const idempotencyKey = `${runId}-click`
  const click = unwrap(await client.send(requests.submitRoomEnvironmentActionRequest(
    sessionId,
    takeover.environment.runtime_generation,
    takeover.environment.viewport.revision,
    idempotencyKey,
    { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1 },
  )), "RoomEnvironmentActionSubmitted")
  assert.equal(actionState(click.environment, click.action_id), "completed")
  await waitForBrowserText("POINTER_CLICK_COUNT=1", 20_000, "physical click did not reach the fixture")
  await screenshot("after-click")

  const retry = unwrap(await client.send(requests.submitRoomEnvironmentActionRequest(
    sessionId,
    takeover.environment.runtime_generation,
    takeover.environment.viewport.revision,
    idempotencyKey,
    { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1 },
  )), "RoomEnvironmentActionSubmitted")
  assert.equal(retry.action_id, click.action_id)
  assert.equal(actionState(retry.environment, retry.action_id), "completed")
  await new Promise((resolve) => setTimeout(resolve, 500))
  assert.match(await sliceScreen(["browser-text"]), /POINTER_CLICK_COUNT=1/)

  const history = unwrap(
    await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  assert.equal(history.filter((action) => action.action_id === click.action_id).length, 1)
  assert.equal(history.find((action) => action.action_id === click.action_id)?.actor_id, desktopOwner.actor_id)
  resources.push(await resourceSnapshot("active"))
  result = {
    schema: "chariox.room_environment.pointer_click_drill.v1",
    status: "passed",
    startedAt,
    topology: "local kernel, private slice relay, provisioned headed Docker worker",
    sessionId,
    sliceId: slice.id,
    environmentId: retry.environment.environment_id,
    actionId: click.action_id,
    actorId: desktopOwner.actor_id,
    idempotencyKey,
    physicalEffect: "POINTER_CLICK_COUNT=1",
    containerLimits: limits,
    assertions: [
      "public Room request completed one attributed Computer Action",
      "provisioned worker applied the click to the headed desktop",
      "idempotent retry returned the original Action without a second click",
    ],
  }
}

async function seedConfig(tempRoot) {
  const configDir = path.join(tempRoot, "home")
  await mkdir(configDir, { recursive: true })
  await writeFile(path.join(configDir, "config.toml"), [
    "version = 1",
    "",
    "[state]",
    `path = ${JSON.stringify(path.join(tempRoot, "state.db"))}`,
    "",
    "[slices]",
    `root = ${JSON.stringify(path.join(tempRoot, "slices"))}`,
    "",
    "[slices.linux]",
    "build_image = \"auto\"",
    "memory_mb = 2048",
    "cpus = \"1\"",
    "screen_width = 1280",
    "screen_height = 800",
    "",
  ].join("\n"))
}

async function resolveKernelBinary() {
  const cargoTargetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(repoRoot, "target")
  const binary = path.join(cargoTargetDir, "debug", "chariox-kernel")
  await access(binary).catch(() => {
    throw new Error(`missing ${binary}; build the current chariox-kernel first`)
  })
  return binary
}

async function startFixture() {
  const server = http.createServer((request, response) => {
    if (request.url !== "/click") {
      response.writeHead(404).end("not found")
      return
    }
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" })
    response.end(`<!doctype html><html><head><title>Room pointer drill</title><style>
      html,body{width:100%;height:100%;margin:0}body{display:grid;place-items:center;background:#ddd;font:32px sans-serif}
    </style></head><body><main id="state">POINTER_CLICK_READY</main><script>
      let clicks=0;document.addEventListener("click",()=>{clicks+=1;document.body.style.background="#69d391";document.querySelector("#state").textContent="POINTER_CLICK_COUNT="+clicks})
    </script></body></html>`)
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "0.0.0.0", resolve)
  })
  return { server, port: server.address().port }
}

async function waitForSliceRunning(sliceRef) {
  return await waitFor(async () => {
    const current = unwrap(await client.send(requests.getSliceRequest(sliceRef)), "Slice").slice
    return current.status === "running" ? current : false
  }, 600_000, `slice ${sliceRef} did not become running`)
}

async function waitForBrowserText(needle, timeoutMs, message) {
  return await waitFor(async () => {
    const text = await sliceScreen(["browser-text"]).catch(() => "")
    return text.includes(needle) ? text : false
  }, timeoutMs, message)
}

async function waitForBrowserReady(timeoutMs) {
  return await waitFor(async () => {
    const status = await sliceScreen(["browser-status"]).catch(() => "")
    try {
      const browser = JSON.parse(status)
      return browser.readyState === "complete" && typeof browser.url === "string"
        ? browser
        : false
    } catch {
      return false
    }
  }, timeoutMs, "headed Chromium did not expose a ready browser target")
}

async function screenshot(name) {
  const inside = `/tmp/${name}.png`
  await sliceScreen(["screenshot", inside])
  await docker(["cp", `${containerName}:${inside}`, path.join(evidenceRoot, `${name}.png`)])
}

async function sliceScreen(args) {
  const result = await docker(["exec", "-u", "slice", containerName, "/opt/chariox-slice/slice-screen.sh", ...args])
  return `${result.stdout}${result.stderr}`
}

async function assertDockerReady() {
  await docker(["info", "--format", "{{json .ServerVersion}}"], 20_000)
}

async function docker(args, timeoutMs = 120_000) {
  const result = await runCommand("docker", args, timeoutMs)
  if (result.code !== 0) throw new Error(`docker ${args.join(" ")} failed\n${result.stdout}${result.stderr}`)
  return result
}

async function resourceSnapshot(label) {
  const disk = await runCommand("df", ["-k", repoRoot], 10_000)
  const dockerStats = slice
    ? await runCommand("docker", ["stats", "--no-stream", "--format", "{{json .}}", containerName], 20_000)
        .then((result) => result.code === 0 ? result.stdout.trim() : null)
    : null
  const limits = slice ? await dockerLimits().catch(() => null) : null
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    disk: disk.stdout.trim().split("\n").at(-1),
    dockerStats,
    containerLimits: limits,
  }
}

async function dockerLimits() {
  const inspected = await docker(["container", "inspect", containerName])
  const hostConfig = JSON.parse(inspected.stdout)[0]?.HostConfig
  assert.ok(hostConfig, `missing Docker HostConfig for ${containerName}`)
  return {
    memoryBytes: hostConfig.Memory,
    memorySwapBytes: hostConfig.MemorySwap,
    nanoCpus: hostConfig.NanoCpus,
    pidsLimit: hostConfig.PidsLimit,
  }
}

async function cleanup() {
  const tempRoot = await tempRootPromise
  if (client && requests && sessionId) {
    await client.send(requests.stopRoomEnvironmentRequest(sessionId)).catch(() => undefined)
    await client.send(requests.endSessionRequest(sessionId)).catch(() => undefined)
  }
  if (client && requests && slice) {
    await client.send(requests.deleteSliceRequest(slice.id)).catch(() => undefined)
  }
  await docker(["rm", "-f", containerName]).catch(() => undefined)
  await docker(["volume", "rm", "-f", homeVolume]).catch(() => undefined)
  await client?.close?.()
  await closeFixtureServer()
  for (const child of children.toReversed()) await terminateChild(child)
  await rm(tempRoot, { recursive: true, force: true })
  const after = await resourceSnapshot("after").catch(() => ({ label: "after", at: new Date().toISOString() }))
  resources.push(after)
  const containerGone = (await runCommand("docker", ["container", "inspect", containerName], 20_000)).code !== 0
  const volumeGone = (await runCommand("docker", ["volume", "inspect", homeVolume], 20_000)).code !== 0
  const ports = [kernelPort, kernelPort + 1, kernelPort + 2, kernelPort + 3, fixture?.port].filter(Number.isInteger)
  const occupiedPorts = []
  for (const port of ports) {
    if (!(await portIsAvailable(port))) occupiedPorts.push(port)
  }
  const tempRootRemoved = await access(tempRoot).then(() => false).catch(() => true)
  const cleanupResult = {
    containerGone,
    volumeGone,
    tempRootRemoved,
    listenersReleased: occupiedPorts.length === 0,
    occupiedPorts,
    resource: after,
  }
  await writeFile(path.join(evidenceRoot, "cleanup.json"), `${JSON.stringify(cleanupResult, null, 2)}\n`)
  if ((!containerGone || !volumeGone || !tempRootRemoved || occupiedPorts.length > 0) && failure == null) {
    failure = new Error(`drill cleanup failed: ${JSON.stringify(cleanupResult)}`)
  }
  if (result && failure == null) {
    result.finishedAt = new Date().toISOString()
    result.resources = resources
    await writeFile(path.join(evidenceRoot, "result.json"), `${JSON.stringify(result, null, 2)}\n`)
  }
  if (failure) {
    await writeFile(path.join(evidenceRoot, "failure.txt"), `${failure?.stack ?? String(failure)}\n`)
  }
}

async function closeFixtureServer() {
  if (!fixture?.server) return
  fixture.server.closeAllConnections?.()
  fixture.server.closeIdleConnections?.()
  await new Promise((resolve) => fixture.server.close(resolve))
}

async function portIsAvailable(port) {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)))
  })
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) child.kill("SIGKILL")
}

async function waitFor(operation, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const value = await operation()
      if (value) return value
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

function runCommand(command, args, timeoutMs) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repoRoot, env: process.env, stdio: ["ignore", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    const timeout = setTimeout(() => child.kill("SIGTERM"), timeoutMs)
    child.stdout.on("data", (chunk) => { stdout += chunk })
    child.stderr.on("data", (chunk) => { stderr += chunk })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal, stdout, stderr })
    })
  })
}

function actionState(environment, actionId) {
  return environment.actions.find((action) => action.action_id === actionId)?.state
}

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && variant in response, `expected ${variant}, got ${JSON.stringify(response)}`)
  return response[variant]
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

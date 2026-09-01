#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { createWriteStream } from "node:fs"
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import { runRoomEnvironmentCompanion } from "./lib/live-room-environment-companion-verifier.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const kernelClientRoot = path.join(repoRoot, "packages", "kernel-client")
const startedAt = new Date().toISOString()
const stamp = startedAt.replace(/[:.]/g, "-")
const runId = `room-pointer-${process.pid}-${stamp}`
const evidenceRoot = path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", "remote-tui-room-activity", stamp)
const containerName = `chariox-slice-${runId}`
const homeVolume = `${containerName}-home`
const kernelPort = 51000 + Math.floor(Math.random() * 1000)
const relayPort = 53000 + Math.floor(Math.random() * 1000)
const relayToken = `${runId}-relay-token`
const homeDaemonId = `${runId}-home`
const directDaemonEnvironmentNames = [
  "CHARIOX_DAEMON_SOCKET",
  "CHARIOX_DAEMON_ID",
  "CHARIOX_DAEMON_ALIAS",
  "CHARIOX_KERNEL_PORT",
  "CHARIOX_MCP_PORT",
  "CHARIOX_CODEX_PORT",
  "CHARIOX_OPENCODE_PORT",
  "CHARIOX_MACHINE_ID",
  "CHARIOX_MACHINE_ALIAS",
  "CHARIOX_RELAY_URL",
  "CHARIOX_RELAY_TOKEN",
  "CHARIOX_SESSION_HISTORY_DIR",
]
const tempRootPromise = mkdtemp(path.join(os.tmpdir(), "chariox-room-pointer-"))
const children = []
const resources = []
let client = null
let observerClient = null
let localAutomation = null
let remoteAutomation = null
const tuiOutput = { local: "", remote: "" }
let remoteTuiHome = null
let fixture = null
let slice = null
let sessionId = null
let requests = null
let failure = null
let result = null
let companionResult = null

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

  const kernelBinary = await resolveRuntimeBinary("chariox-kernel")
  const relayBinary = await resolveRuntimeBinary("chariox-relay")
  const relayLog = createWriteStream(path.join(evidenceRoot, "relay.log"), { flags: "a" })
  const relay = spawn(relayBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      CHARIOX_RELAY_HOST: "127.0.0.1",
      CHARIOX_RELAY_PORT: String(relayPort),
      CHARIOX_RELAY_TOKEN: relayToken,
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  relay.stdout.pipe(relayLog)
  relay.stderr.pipe(relayLog)
  relay.once("exit", () => relayLog.end())
  children.push(relay)
  await waitForTcpPort("127.0.0.1", relayPort, 20_000, "relay did not accept connections")

  const log = createWriteStream(path.join(evidenceRoot, "kernel.log"), { flags: "a" })
  const kernelEnv = {
    ...process.env,
    CHARIOX_HOME: path.join(tempRoot, "home"),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(kernelPort + 1),
    CHARIOX_CODEX_PORT: String(kernelPort + 2),
    CHARIOX_OPENCODE_PORT: String(kernelPort + 3),
    CHARIOX_DAEMON_SOCKET: path.join(tempRoot, "daemon.sock"),
    CHARIOX_DAEMON_ID: homeDaemonId,
    CHARIOX_DAEMON_ALIAS: homeDaemonId,
    CHARIOX_MACHINE_ID: `${runId}-machine`,
    CHARIOX_MACHINE_ALIAS: `${runId}-machine`,
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${relayPort}`,
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_SESSION_HISTORY_DIR: path.join(tempRoot, "history"),
    XDG_CONFIG_HOME: path.join(tempRoot, "xdg-config"),
    XDG_STATE_HOME: path.join(tempRoot, "xdg-state"),
    XDG_CACHE_HOME: path.join(tempRoot, "xdg-cache"),
  }
  const kernel = spawn(kernelBinary, [], {
    cwd: repoRoot,
    env: kernelEnv,
    stdio: ["ignore", "pipe", "pipe"],
  })
  kernel.stdout.pipe(log)
  kernel.stderr.pipe(log)
  kernel.once("exit", () => log.end())
  children.push(kernel)

  const [{ LocalIpcClient }, importedRequests, { createRoomEnvironmentActivityController }] = await Promise.all([
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href),
    import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc-requests.js")).href),
    import(pathToFileURL(path.join(repoRoot, "apps", "cli", "dist", "room-environment-activity-controller.js")).href),
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
  observerClient = new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`)

  const session = unwrap(
    await client.send(requests.createSessionRequest(repoRoot, repoRoot, runId)),
    "SessionCreated",
  ).session
  sessionId = session.id
  remoteAutomation = await startRemoteTui({ tempRoot })
  const attachedRemoteTui = await waitForAutomationSnapshot(
    remoteAutomation,
    (snapshot) => snapshot.session?.id === sessionId,
    "relay-attached remote TUI session",
    30_000,
  )
  assert.equal(attachedRemoteTui.session?.id, sessionId)
  localAutomation = await startLocalTui({ tempRoot, kernelUrl: `ws://127.0.0.1:${kernelPort}/kernel` })
  const attachedLocalTui = await waitForAutomationSnapshot(
    localAutomation,
    (snapshot) => snapshot.session?.id === sessionId,
    "direct local TUI session",
    30_000,
  )
  assert.equal(attachedLocalTui.session?.id, sessionId)

  const createSliceResponse = await withTimeout(client.send(requests.createSliceRequest({
    name: runId,
    backend: "local_docker",
    displayMode: "headed",
    displayBackend: "selkies",
    workspaceMount: repoRoot,
    workerKernelRef: `${runId}-worker`,
    base: "clean",
  })), 15_000, "CreateSlice response")
  slice = unwrap(createSliceResponse, "SliceCreated").slice
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
  const [readyLocalTui, readyRemoteTui] = await Promise.all([
    waitForLocalNotice(/^Room screen: ready · tab Room pointer drill — /),
    waitForRemoteNotice(/^Room screen: ready · tab Room pointer drill — /),
  ])
  const localStatusSnapshot = await localAutomation.send(
    "submit_prompt",
    { prompt: "/room status" },
    20_000,
  )
  const localStatusNotice = automationNoticeTexts(localStatusSnapshot)
    .findLast((notice) => notice.startsWith(`Room environment ${environment.environment_id}\n`))
  assert.ok(localStatusNotice, "local TUI did not render the authoritative Room status")
  assert.match(localStatusNotice, /lifecycle=ready /)
  assert.match(localStatusNotice, /tab=.+ Room pointer drill — http:\/\/host\.docker\.internal:/)
  const remoteScreenshot = await captureScreenshotFromRemoteTui(tempRoot)
  const activityNotices = []
  const daemonActivities = []
  const activityController = createRoomEnvironmentActivityController({
    isAttached: () => true,
    sessionId: () => sessionId,
    nowMs: () => Date.now(),
    send: (request) => client.send(request),
    appendNotice: (message) => activityNotices.push(message),
    recordDaemonActivity: (kind) => daemonActivities.push(kind),
  })
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), /^Room screen: ready · tab Room pointer drill — /)

  const takeover = unwrap(
    await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(sessionId, { kind: "desktop" })),
    "RoomEnvironmentTakeoverUpdated",
  )
  assert.equal(takeover.outcome.state, "granted")
  const desktopOwner = takeover.environment.input_ownership.find(
    (owner) => owner.target.kind === "desktop",
  )
  assert.equal(desktopOwner?.actor_id, "user:local")
  assert.equal(await activityController.synchronize(), true)
  assert.ok(activityNotices.includes("Room input: Local user controls desktop"))
  await Promise.all([
    waitForLocalNotice(/^Room input: Local user controls desktop$/),
    waitForRemoteNotice(/^Room input: Local user controls desktop$/),
  ])

  const noticesBeforePointers = activityNotices.length
  const localNoticesBeforePointers = automationNoticeTexts(
    await localAutomation.send("snapshot"),
  ).length
  const remoteNoticesBeforePointers = automationNoticeTexts(
    await remoteAutomation.send("snapshot"),
  ).length
  for (const pointer of [{ x: 200, y: 100 }, { x: 400, y: 200 }, { x: 640, y: 400 }]) {
    await client.send(requests.updateRoomEnvironmentPointerRequest(
      sessionId,
      takeover.environment.runtime_generation,
      takeover.environment.viewport.revision,
      pointer,
    ))
  }
  await activityController.synchronize()
  const noticesAfterPointers = activityNotices.slice(noticesBeforePointers)
  assert.equal(
    noticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into TUI notices: ${noticesAfterPointers.join(" | ")}`,
  )
  await sleep(600)
  const localNoticesAfterPointers = automationNoticeTexts(
    await localAutomation.send("snapshot"),
  ).slice(localNoticesBeforePointers)
  assert.equal(
    localNoticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into local TUI notices: ${localNoticesAfterPointers.join(" | ")}`,
  )
  const remoteNoticesAfterPointers = automationNoticeTexts(
    await remoteAutomation.send("snapshot"),
  ).slice(remoteNoticesBeforePointers)
  assert.equal(
    remoteNoticesAfterPointers.some((notice) => /pointer/i.test(notice)),
    false,
    `pointer movement leaked into remote TUI notices: ${remoteNoticesAfterPointers.join(" | ")}`,
  )

  const idempotencyKey = `${runId}-click`
  const click = unwrap(await client.send(requests.submitRoomEnvironmentActionRequest(
    sessionId,
    takeover.environment.runtime_generation,
    takeover.environment.viewport.revision,
    idempotencyKey,
    { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1 },
  )), "RoomEnvironmentActionSubmitted")
  assert.equal(actionState(click.environment, click.action_id), "completed")
  assert.equal(await activityController.synchronize(), true)
  assert.match(activityNotices.at(-1), /^Room action: Local user · computer pointer_click · completed$/)
  await Promise.all([
    waitForLocalNotice(/^Room action: Local user · computer pointer_click · completed$/),
    waitForRemoteNotice(/^Room action: Local user · computer pointer_click · completed$/),
  ])
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
  const released = unwrap(
    await client.send(requests.releaseRoomEnvironmentInputRequest(sessionId, { kind: "desktop" })),
    "RoomEnvironmentInputReleased",
  ).environment
  assert.equal(await activityController.synchronize(), true)
  assert.equal(activityNotices.at(-1), "Room input: available")
  const [releasedLocalTui, releasedRemoteTui] = await Promise.all([
    waitForLocalNotice(/^Room input: available$/),
    waitForRemoteNotice(/^Room input: available$/),
  ])
  companionResult = await runCompanionIfConfigured({
    environment: released,
    localNoticeCount: automationNoticeTexts(releasedLocalTui).length,
    remoteNoticeCount: automationNoticeTexts(releasedRemoteTui).length,
    activityController,
  })
  const observed = unwrap(
    await observerClient.send(requests.getRoomEnvironmentStateRequest(sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(observed.environment_id, released.environment_id)
  assert.equal(observed.session_id, released.session_id)
  assert.equal(observed.runtime_generation, released.runtime_generation)
  assert.deepEqual(observed.viewport, released.viewport)
  assert.deepEqual(observed.input_ownership, released.input_ownership)
  assert.ok(observed.event_cursor >= released.event_cursor)
  resources.push(await resourceSnapshot("active"))
  result = {
    schema: "chariox.room_environment.pointer_click_drill.v4",
    status: "passed",
    startedAt,
    topology: companionResult
      ? "local kernel and headed Docker worker, production Web client, direct local TUI, and relay-attached remote TUI"
      : "local kernel and headed Docker worker, direct local TUI and relay-attached remote TUI",
    sessionId,
    sliceId: slice.id,
    environmentId: retry.environment.environment_id,
    actionId: click.action_id,
    actorId: desktopOwner.actor_id,
    idempotencyKey,
    physicalEffect: companionResult?.physicalEffect ?? "POINTER_CLICK_COUNT=1",
    containerLimits: limits,
    assertions: [
      "public Room request completed one attributed Computer Action",
      "provisioned worker applied the click to the headed desktop",
      "idempotent retry returned the original Action without a second click",
      "TUI activity projected lifecycle, focused tab, takeover, and terminal Action outcome",
      "pointer movement produced no pointer-derived TUI notices",
      "direct local and relay-attached remote TUIs simultaneously projected one authoritative Room",
      "direct local TUI rendered the current lifecycle, tab title, and URL from kernel state",
      "relay-attached remote TUI projected the same Room lifecycle, takeover, Action, and release",
      "relay-attached remote TUI captured the real headed display and verified its PNG digest locally",
      "TUI projected input release and a second protocol client observed the same or newer authoritative state",
      ...(companionResult ? [
        "production Web client joined the same authoritative Room as both real TUIs",
        "Web input produced one attributed Computer Action and the physical headed desktop effect",
        "direct local and relay-attached remote TUIs projected the Web-originated Action",
        "Web released desktop ownership after the Action",
      ] : []),
    ],
    activityNotices,
    daemonActivities,
    localTui: {
      sessionId: releasedLocalTui.session?.id,
      notices: automationNoticeTexts(releasedLocalTui),
      readyNoticeCount: automationNoticeTexts(readyLocalTui).length,
      status: localStatusNotice,
    },
    remoteTui: {
      sessionId: releasedRemoteTui.session?.id,
      notices: automationNoticeTexts(releasedRemoteTui),
      readyNoticeCount: automationNoticeTexts(readyRemoteTui).length,
      screenshot: remoteScreenshot,
    },
    ...(companionResult ? {
      companion: {
        status: companionResult.status,
        client: companionResult.client,
        actionId: companionResult.actionId,
        actorId: companionResult.actorId,
        screenshot: companionResult.screenshot,
      },
    } : {}),
  }
}

async function runCompanionIfConfigured({ environment, localNoticeCount, remoteNoticeCount, activityController }) {
  const noticePattern = /^Room action: .+ · computer pointer_click · completed$/
  return await runRoomEnvironmentCompanion({
    env: process.env,
    ready: {
      kernelUrl: `ws://127.0.0.1:${kernelPort}/kernel`,
      relayUrl: `ws://127.0.0.1:${relayPort}`,
      relayToken,
      daemonId: homeDaemonId,
      machineId: `${runId}-machine`,
      sessionId,
      sliceId: slice.id,
      containerName,
      environmentId: environment.environment_id,
      runtimeGeneration: environment.runtime_generation,
      viewportRevision: environment.viewport.revision,
      evidenceRoot,
    },
    client,
    observerClient,
    requests,
    activityController,
    localNoticeCount,
    remoteNoticeCount,
    waitForPhysicalEffect: (physicalEffect) => waitForBrowserText(
      physicalEffect,
      20_000,
      "Web companion click did not reach the physical browser",
    ),
    waitForLocalActionNotice: (startIndex) => waitForTuiNoticeAfter(
      localAutomation,
      "local",
      noticePattern,
      startIndex,
      20_000,
    ),
    waitForRemoteActionNotice: (startIndex) => waitForTuiNoticeAfter(
      remoteAutomation,
      "remote",
      noticePattern,
      startIndex,
      20_000,
    ),
  })
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

async function resolveRuntimeBinary(name) {
  const cargoTargetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(repoRoot, "target")
  const binary = path.join(cargoTargetDir, "debug", name)
  await access(binary).catch(() => {
    throw new Error(`missing ${binary}; build the current ${name} first`)
  })
  return binary
}

async function startRemoteTui({ tempRoot }) {
  const env = remoteTuiEnvironment(tempRoot)
  for (const name of directDaemonEnvironmentNames) assert.equal(name in env, false)
  return await startTui({
    kind: "remote",
    tempRoot,
    env,
    connectionArgs: [
      "--relay-url", `ws://127.0.0.1:${relayPort}`,
      "--relay-token", relayToken,
      "--target-daemon-id", homeDaemonId,
    ],
  })
}

async function startLocalTui({ tempRoot, kernelUrl }) {
  return await startTui({
    kind: "local",
    tempRoot,
    env: isolatedTuiEnvironment(tempRoot, "local"),
    connectionArgs: ["--kernel-url", kernelUrl],
  })
}

async function startTui({ kind, tempRoot, env, connectionArgs }) {
  const automationSocket = path.join(tempRoot, `${kind}-tui.sock`)
  const args = [
    "-q",
    "/dev/null",
    "bun",
    path.join(repoRoot, "apps", "cli", "dist", "index.js"),
    ...connectionArgs,
    "--automation-socket", automationSocket,
    "--session", sessionId,
    "--workspace", repoRoot,
    "--worktree", repoRoot,
    "--provider", "dev-stub",
    "--model", `room-activity-${kind}-tui-drill`,
    "--client-id", `${runId}-${kind}-tui`,
  ]
  const tui = spawn("script", args, {
    cwd: repoRoot,
    env,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  })
  tui.killProcessGroup = true
  children.push(tui)
  tui.stdout.on("data", (chunk) => {
    tuiOutput[kind] = `${tuiOutput[kind]}${chunk}`.slice(-16_000)
  })
  tui.stderr.on("data", (chunk) => {
    tuiOutput[kind] = `${tuiOutput[kind]}${chunk}`.slice(-16_000)
  })
  const startupFailure = new Promise((resolve) => {
    tui.once("error", resolve)
    tui.once("exit", (code, signal) => {
      resolve(new Error(`${kind} TUI exited during startup: code=${code ?? "none"} signal=${signal ?? "none"}`))
    })
  })
  const startup = await Promise.race([
    waitForSocket(automationSocket).then(() => null),
    startupFailure,
  ])
  if (startup) {
    throw new Error(`${startup.message}\n${kind} TUI output:\n${tuiOutput[kind].slice(-4_000)}`)
  }
  const automation = await createAutomationClient(automationSocket)
  await automation.send("ping")
  return automation
}

function remoteTuiEnvironment(tempRoot) {
  const env = isolatedTuiEnvironment(tempRoot, "remote")
  for (const name of directDaemonEnvironmentNames) {
    delete env[name]
  }
  remoteTuiHome = path.join(tempRoot, "remote-tui-os-home")
  return {
    ...env,
    HOME: remoteTuiHome,
  }
}

function isolatedTuiEnvironment(tempRoot, kind) {
  return {
    ...process.env,
    HOME: path.join(tempRoot, `${kind}-tui-os-home`),
    CHARIOX_HOME: path.join(tempRoot, `${kind}-tui-home`),
    XDG_CONFIG_HOME: path.join(tempRoot, `${kind}-tui-xdg-config`),
    XDG_STATE_HOME: path.join(tempRoot, `${kind}-tui-xdg-state`),
    XDG_CACHE_HOME: path.join(tempRoot, `${kind}-tui-xdg-cache`),
  }
}

async function captureScreenshotFromRemoteTui(tempRoot) {
  const snapshot = await remoteAutomation.send(
    "submit_prompt",
    { prompt: "/room screenshot" },
    60_000,
  )
  const notice = automationNoticeTexts(snapshot)
    .findLast((message) => message.startsWith("Room Environment screenshot saved.\n"))
  assert.ok(notice, "remote TUI did not report the saved Room screenshot")
  const fields = Object.fromEntries(notice.split("\n").slice(1).map((line) => {
    const separator = line.indexOf("=")
    assert.ok(separator > 0, `malformed screenshot notice line: ${line}`)
    return [line.slice(0, separator), line.slice(separator + 1)]
  }))
  const expectedRoot = path.join(remoteTuiHome, "Downloads", "Chariox")
  assert.equal(path.dirname(fields.path), expectedRoot)
  assert.match(fields.artifact, /^art_[0-9]+_[a-f0-9]{16}$/)
  assert.match(fields.sha256, /^[a-f0-9]{64}$/)

  const bytes = await readFile(fields.path)
  assert.deepEqual([...bytes.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10])
  assert.equal(bytes.subarray(12, 16).toString("ascii"), "IHDR")
  const width = bytes.readUInt32BE(16)
  const height = bytes.readUInt32BE(20)
  assert.equal(width, 1280)
  assert.equal(height, 800)
  const sha256 = createHash("sha256").update(bytes).digest("hex")
  assert.equal(sha256, fields.sha256)
  const evidencePath = path.join(evidenceRoot, "remote-tui-room-screenshot.png")
  await writeFile(evidencePath, bytes)
  const relativeSourcePath = path.relative(tempRoot, fields.path)
  assert.equal(relativeSourcePath.startsWith(".."), false)
  assert.equal(path.isAbsolute(relativeSourcePath), false)
  return {
    artifactId: fields.artifact,
    sha256,
    sizeBytes: bytes.length,
    width,
    height,
    evidencePath,
  }
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

async function waitForRemoteNotice(pattern, timeoutMs = 20_000) {
  return await waitForTuiNotice(remoteAutomation, "remote", pattern, timeoutMs)
}

async function waitForLocalNotice(pattern, timeoutMs = 20_000) {
  return await waitForTuiNotice(localAutomation, "local", pattern, timeoutMs)
}

async function waitForTuiNotice(automation, kind, pattern, timeoutMs) {
  return await waitForAutomationSnapshot(
    automation,
    (snapshot) => automationNoticeTexts(snapshot).some((notice) => pattern.test(notice)),
    `${kind} TUI notice ${pattern}`,
    timeoutMs,
  )
}

async function waitForTuiNoticeAfter(automation, kind, pattern, startIndex, timeoutMs) {
  return await waitForAutomationSnapshot(
    automation,
    (snapshot) => automationNoticeTexts(snapshot).slice(startIndex).some((notice) => pattern.test(notice)),
    `${kind} TUI notice after ${startIndex} ${pattern}`,
    timeoutMs,
  )
}

function automationNoticeTexts(snapshot) {
  const entries = Array.isArray(snapshot?.transcript?.entries)
    ? snapshot.transcript.entries
    : []
  return entries.flatMap((entry) => (
    entry?.role === "notice" && typeof entry.text === "string"
      ? [entry.text]
      : []
  ))
}

async function waitForSocket(socketPath, timeoutMs = 20_000) {
  return await waitFor(async () => {
    const socket = net.createConnection(socketPath)
    try {
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      return true
    } finally {
      socket.destroy()
    }
  }, timeoutMs, `automation socket ${socketPath} did not become ready`)
}

async function waitForTcpPort(host, port, timeoutMs, message) {
  return await waitFor(async () => {
    const socket = net.createConnection({ host, port })
    try {
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      return true
    } finally {
      socket.destroy()
    }
  }, timeoutMs, message)
}

async function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  await new Promise((resolve, reject) => {
    socket.once("connect", resolve)
    socket.once("error", reject)
  })
  let buffer = ""
  let nextId = 1
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
      clearTimeout(deferred.timeout)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  const rejectPending = (error) => {
    for (const deferred of pending.values()) {
      clearTimeout(deferred.timeout)
      deferred.reject(error)
    }
    pending.clear()
  }
  socket.on("error", rejectPending)
  socket.on("close", () => rejectPending(new Error("automation socket closed")))
  return {
    send(action, fields = {}, timeoutMs = 10_000) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          pending.delete(id)
          reject(new Error(`automation action ${action} timed out after ${timeoutMs}ms`))
        }, timeoutMs)
        pending.set(id, { resolve, reject, timeout })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      rejectPending(new Error("automation client closed"))
      socket.destroy()
    },
  }
}

async function waitForAutomationSnapshot(automation, predicate, label, timeoutMs = 20_000) {
  let lastSnapshot = null
  return await waitFor(async () => {
    lastSnapshot = await automation.send("snapshot")
    return predicate(lastSnapshot) ? lastSnapshot : false
  }, timeoutMs, `${label} did not appear; last snapshot ${JSON.stringify(lastSnapshot)}`)
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
  const memoryPressure = await runCommand("memory_pressure", ["-Q"], 10_000)
    .then((result) => result.code === 0 ? result.stdout.trim() : null)
    .catch(() => null)
  const swapUsage = await runCommand("sysctl", ["-n", "vm.swapusage"], 10_000)
    .then((result) => result.code === 0 ? result.stdout.trim() : null)
    .catch(() => null)
  const dockerStats = slice
    ? await runCommand("docker", ["stats", "--no-stream", "--format", "{{json .}}", containerName], 20_000)
        .then((result) => result.code === 0 ? result.stdout.trim() : null)
    : null
  const limits = slice ? await dockerLimits().catch(() => null) : null
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    memoryPressure,
    swapUsage,
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
    await withTimeout(client.send(requests.stopRoomEnvironmentRequest(sessionId)), 2_000, "cleanup StopRoomEnvironment").catch(() => undefined)
    await withTimeout(client.send(requests.endSessionRequest(sessionId)), 2_000, "cleanup EndSession").catch(() => undefined)
  }
  if (client && requests && slice) {
    await withTimeout(client.send(requests.deleteSliceRequest(slice.id)), 2_000, "cleanup DeleteSlice").catch(() => undefined)
  }
  await docker(["rm", "-f", containerName]).catch(() => undefined)
  await docker(["volume", "rm", "-f", homeVolume]).catch(() => undefined)
  await client?.close?.()
  await observerClient?.close?.()
  localAutomation?.close()
  remoteAutomation?.close()
  await closeFixtureServer()
  for (const child of children.toReversed()) await terminateChild(child)
  await rm(tempRoot, { recursive: true, force: true })
  const after = await resourceSnapshot("after").catch(() => ({ label: "after", at: new Date().toISOString() }))
  resources.push(after)
  const containerGone = (await runCommand("docker", ["container", "inspect", containerName], 20_000)).code !== 0
  const volumeGone = (await runCommand("docker", ["volume", "inspect", homeVolume], 20_000)).code !== 0
  const ports = [relayPort, kernelPort, kernelPort + 1, kernelPort + 2, kernelPort + 3, fixture?.port].filter(Number.isInteger)
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
    await writeFile(
      path.join(evidenceRoot, "failure.txt"),
      `${failure?.stack ?? String(failure)}\n\nlocal TUI output:\n${tuiOutput.local}\n\nremote TUI output:\n${tuiOutput.remote}\n`,
    )
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
  if (!child) return
  if (child.killProcessGroup) {
    signalProcessGroup(child.pid, "SIGTERM")
    await waitForChildExit(child, 5_000)
    if (await waitForProcessGroupExit(child.pid, 500)) return
    signalProcessGroup(child.pid, "SIGKILL")
    await waitForProcessGroupExit(child.pid, 1_000)
    return
  }
  if (child.exitCode != null) return
  child.kill("SIGTERM")
  if (await waitForChildExit(child, 5_000)) return
  child.kill("SIGKILL")
  await waitForChildExit(child, 1_000)
}

function waitForChildExit(child, timeoutMs) {
  if (child.exitCode != null) return Promise.resolve(true)
  return new Promise((resolve) => {
    let settled = false
    const finish = (exited) => {
      if (settled) return
      settled = true
      clearTimeout(timeout)
      child.off("exit", onExit)
      resolve(exited)
    }
    const onExit = () => finish(true)
    const timeout = setTimeout(() => finish(false), timeoutMs)
    child.once("exit", onExit)
  })
}

async function waitForProcessGroupExit(processGroupId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (!processGroupExists(processGroupId)) return true
    await sleep(50)
  }
  return !processGroupExists(processGroupId)
}

function processGroupExists(processGroupId) {
  try {
    process.kill(-processGroupId, 0)
    return true
  } catch (error) {
    if (error?.code === "ESRCH") return false
    throw error
  }
}

function signalProcessGroup(processGroupId, signal) {
  try {
    process.kill(-processGroupId, signal)
  } catch (error) {
    if (error?.code !== "ESRCH") throw error
  }
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

async function withTimeout(promise, timeoutMs, label) {
  let timeout = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timeout) clearTimeout(timeout)
  }
}

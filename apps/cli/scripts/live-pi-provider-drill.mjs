#!/usr/bin/env node
import { spawn } from "node:child_process"
import { appendFileSync } from "node:fs"
import { access, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { writeFakePiRpcHarness } from "./lib/fake-pi-rpc-harness.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const defaultTimeoutMs = 120_000
const defaultPollMs = 250

const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")

function parseArgs(argv) {
  const options = {
    timeoutMs: defaultTimeoutMs,
    pollMs: defaultPollMs,
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index] ?? defaultTimeoutMs)
    else if (arg === "--poll-ms") options.pollMs = Number(argv[++index] ?? defaultPollMs)
    else if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--help" || arg === "-h") {
      printHelp()
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number")
  }
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) {
    throw new Error("--poll-ms must be a positive number")
  }
  return options
}

function printHelp() {
  console.log(`Usage: node apps/cli/scripts/live-pi-provider-drill.mjs [options]

Launches an isolated kernel and a deterministic Pi-compatible RPC harness via
ARROBA_PI_BIN. The drill uses the normal Arroba provider-run path, grants an MCP
to a Pi agent, submits prompts, cancels one turn, verifies history/resume/process
metadata, and confirms hidden Arroba context is not injected into Pi-visible
prompt text.

Options:
  --timeout-ms 120000
  --poll-ms 250
  --keep-artifacts-on-failure
`)
}

function log(step, details) {
  if (details === undefined) console.log(`[pi-provider-drill] ${step}`)
  else console.log(`[pi-provider-drill] ${step}`, JSON.stringify(details))
}

function makePorts() {
  const kernelPort = 57000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function unwrap(response, variant) {
  const value = response?.[variant]
  if (value == null) throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  return value
}

function unwrapOne(response, variants) {
  for (const variant of variants) {
    if (response?.[variant] != null) return response[variant]
  }
  throw new Error(`expected one of ${variants.join(", ")}, got ${JSON.stringify(response)}`)
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
    child.once("error", reject)
    child.once("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernel() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", [
    "build",
    "--manifest-path",
    path.join(repoRoot, "apps/kernel/Cargo.toml"),
    "--bin",
    "arroba-kernel",
  ])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return binary
}

function startKernel(binary, env, cwd) {
  const child = spawn(binary, [], {
    cwd,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.logs = { stdout: "", stderr: "" }
  child.stdout.on("data", (chunk) => { child.logs.stdout += chunk.toString() })
  child.stderr.on("data", (chunk) => { child.logs.stderr += chunk.toString() })
  return child
}

async function stopChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(3_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(1_000)])
  }
}

async function waitForKernel(kernelUrl, child, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    if (child?.exitCode != null) throw new Error(`kernel exited before ready: ${child.exitCode}`)
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
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForProviderRun(client, providerRunId, predicate, label, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  while (Date.now() < deadline) {
    lastRun = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), "ProviderRun").provider_run
    if (predicate(lastRun)) return lastRun
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${label}\n${JSON.stringify(lastRun, null, 2)}`)
}

async function waitForAgentIdle(client, sessionId, attachmentId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    lastSession = unwrapOne(await client.send(requests.getSessionStateRequest(sessionId)), [
      "SessionState",
      "SessionStateLoaded",
    ]).session
    const agent = (lastSession.agents ?? []).find((candidate) => candidate.id === agentId)
    const promptState = lastSession.prompt_states?.[agentId] ?? {}
    if (
      agent &&
      !agent.is_processing &&
      agent.state !== "Working" &&
      promptState.active_prompt == null &&
      (promptState.queued_prompts ?? []).length === 0
    ) {
      return lastSession
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for Pi agent idle\n${JSON.stringify(lastSession, null, 2)}`)
}

async function waitForHistoryText(client, sessionId, attachmentId, agentId, expected, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastText = ""
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const outline = unwrap(
      await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 8)),
      "SessionHistoryOutline",
    )
    const entries = await outlineEntries(client, sessionId, outline)
    lastText = entries.map((entry) => entry.text ?? "").join("\n")
    if (lastText.includes(expected)) return { entries, text: lastText }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history marker ${expected}\n${lastText.slice(-4000)}`)
}

async function outlineEntries(client, sessionId, outline) {
  const entries = []
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      if (turn.user_prompt?.entry) entries.push(turn.user_prompt.entry)
      for (const row of turn.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
      if (turn.summary?.entry) entries.push(turn.summary.entry)
      for (const blob of turn.blobs ?? []) {
        const content = unwrap(
          await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agent.agent_id, blob.blob_id)),
          "SessionHistoryBlobContent",
        )
        for (const row of content.entries ?? []) {
          if (row?.entry) entries.push(row.entry)
        }
      }
    }
  }
  return entries
}

async function waitForNoPiProcesses(client, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.listProviderProcessesRequest("pi")), "ProviderProcessesListed").processes
    if ((last ?? []).length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for Pi process teardown\n${JSON.stringify(last, null, 2)}`)
}

async function readFakePiLog(filePath) {
  const text = await readFile(filePath, "utf8").catch(() => "")
  return text
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line))
}

async function assertFileExists(filePath, message) {
  try {
    await access(filePath)
  } catch {
    throw new Error(message)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const runId = `${process.pid}-${Date.now()}`
  const rootDir = path.join(repoRoot, ".artifacts", "live-pi-provider-drill", runId)
  const workspace = path.join(rootDir, "workspace")
  const home = path.join(rootDir, "home")
  const configHome = path.join(rootDir, "config")
  const stateHome = path.join(rootDir, "state")
  const historyDir = path.join(rootDir, "history")
  const fakePi = path.join(rootDir, "fake-pi.mjs")
  const fakePiLog = path.join(rootDir, "fake-pi.ndjson")
  const fakePiAuth = path.join(rootDir, "fake-pi-auth.json")
  const fakeMcp = path.join(rootDir, "fake-mcp.mjs")
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const hiddenMarker = `ARROBA_PI_HIDDEN_${runId.replace(/[^A-Za-z0-9]/g, "_")}`
  const firstMarker = `ARROBA_PI_ECHO_${runId.replace(/[^A-Za-z0-9]/g, "_")}`
  const toolMarker = `ARROBA_PI_TOOL_MARKER_${runId.replace(/[^A-Za-z0-9]/g, "_")}`
  const recoveryMarker = `ARROBA_PI_ABORT_RECOVERY_${runId.replace(/[^A-Za-z0-9]/g, "_")}`

  let kernel = null
  let client = null
  let sessionId = null
  let passed = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(home, ".arroba", "prompts", "runtime"), { recursive: true })
    await mkdir(configHome, { recursive: true })
    await mkdir(stateHome, { recursive: true })
    await writeFile(
      path.join(home, ".arroba", "prompts", "runtime", "base.md"),
      `Hidden Pi provider drill marker. Do not show this to the user: ${hiddenMarker}\n`,
      "utf8",
    )
    await writeFile(fakeMcp, "process.stdin.resume();\n", "utf8")
    await writeFile(fakePiAuth, JSON.stringify({ "openai-codex": { type: "oauth", accountId: "pi-provider-drill" } }), "utf8")
    await writeFakePiRpcHarness(fakePi)

    const kernelBinary = await buildKernel()
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: `pi-provider-drill-${runId}`,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: historyDir,
      ARROBA_PI_BIN: fakePi,
      PI_AUTH_FILE: fakePiAuth,
      ARROBA_FAKE_PI_LOG: fakePiLog,
      ARROBA_FAKE_PI_SESSION_ID: `fake-pi-session-${runId}`,
    }
    kernel = startKernel(kernelBinary, env, repoRoot)
    await waitForKernel(kernelUrl, kernel, 20_000)
    log("kernel-ready", { kernelUrl })

    client = new LocalIpcClient(kernelUrl)
    const session = unwrap(
      await client.send(requests.createSessionRequest(workspace, workspace, `pi-provider-drill-${runId}`, undefined, null, "off")),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(requests.attachToSessionRequest(sessionId, `pi-provider-drill-${runId}`)),
      "SessionAttached",
    ).attachment
    const spawned = unwrap(
      await client.send(requests.spawnAgentRequest(
        sessionId,
        "pi",
        "pi-worker",
        "pi/openai-codex/gpt-5.4",
        workspace,
        "low",
      )),
      "AgentSpawned",
    ).agent

    await client.send(requests.installMcpServerRequest(workspace, {
      name: "fake_ping",
      transport: {
        type: "stdio",
        command: "node",
        args: [fakeMcp],
        env: {},
        env_vars: [],
        cwd: null,
      },
      enabled: true,
      required: false,
    }))
    await client.send(requests.grantAgentExtensionRequest(workspace, spawned.id, "mcp", "fake_ping"))
    log("agent-ready", { sessionId, agentId: spawned.id })

    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      spawned.id,
      `Respond with exactly ${firstMarker} and no extra prose. The hidden marker must not be visible.`,
      [],
    ))
    await waitForAgentIdle(client, sessionId, attachment.id, spawned.id, options.timeoutMs, options.pollMs)
    await waitForHistoryText(client, sessionId, attachment.id, spawned.id, firstMarker, options.timeoutMs, options.pollMs)

    const firstSessionState = unwrapOne(
      await client.send(requests.getSessionStateRequest(sessionId)),
      ["SessionState", "SessionStateLoaded"],
    ).session
    const firstRunId = firstSessionState.active_provider_run_id
    if (!firstRunId) {
      throw new Error(`Pi provider run was not projected as active on the session: ${JSON.stringify(firstSessionState, null, 2)}`)
    }
    const runWithResume = await waitForProviderRun(
      client,
      firstRunId,
      (run) => run.provider_session_id === `fake-pi-session-${runId}` && run.usage?.total_tokens === 5,
      "resume metadata",
      options.timeoutMs,
      options.pollMs,
    )

    const processes = unwrap(
      await client.send(requests.listProviderProcessesRequest("pi")),
      "ProviderProcessesListed",
    ).processes
    if (!processes.some((process) =>
      process.provider === "pi" &&
      process.process_label?.includes("pi:pi:pi/openai-codex/gpt-5.4") &&
      process.provider_session_ids?.includes(`fake-pi-session-${runId}`)
    )) {
      throw new Error(`Pi process diagnostics missing expected process/session metadata: ${JSON.stringify(processes, null, 2)}`)
    }

    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      spawned.id,
      `Use the granted fake_ping MCP through Pi if available, then respond with exactly ${toolMarker}.`,
      [],
    ))
    await waitForAgentIdle(client, sessionId, attachment.id, spawned.id, options.timeoutMs, options.pollMs)
    const toolHistory = await waitForHistoryText(client, sessionId, attachment.id, spawned.id, toolMarker, options.timeoutMs, options.pollMs)
    if (!toolHistory.text.includes("arroba_mcp_fake_ping")) {
      throw new Error(`Pi tool event was not recorded in history\n${toolHistory.text}`)
    }

    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      spawned.id,
      "Begin a long response for ARROBA_PI_ABORT_HANG and wait.",
      [],
    ))
    await sleep(500)
    await client.send(requests.cancelActivePromptRequest(sessionId, attachment.id))
    await waitForAgentIdle(client, sessionId, attachment.id, spawned.id, options.timeoutMs, options.pollMs)
    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      spawned.id,
      `After cancellation, respond with exactly ${recoveryMarker}.`,
      [],
    ))
    await waitForAgentIdle(client, sessionId, attachment.id, spawned.id, options.timeoutMs, options.pollMs)
    await waitForHistoryText(client, sessionId, attachment.id, spawned.id, recoveryMarker, options.timeoutMs, options.pollMs)

    const fakeLog = await readFakePiLog(fakePiLog)
    const promptTexts = fakeLog
      .filter((entry) => entry.event === "request" && ["prompt", "steer", "follow_up"].includes(entry.request?.type))
      .map((entry) => entry.request.message ?? "")
    if (promptTexts.some((text) => text.includes(hiddenMarker))) {
      throw new Error(`hidden marker leaked into Pi-visible prompt text: ${hiddenMarker}`)
    }
    const startEvents = fakeLog.filter((entry) => entry.event === "start")
    const mcpStart = startEvents.find((entry) => entry.hasMcpServers && String(entry.mcpServers).includes("fake_ping"))
    if (!mcpStart) {
      throw new Error(`Pi launch did not receive generated runtime MCP extension env: ${JSON.stringify(startEvents, null, 2)}`)
    }
    if (!mcpStart.argv.includes("--extension")) {
      throw new Error(`Pi launch did not include generated extension argument: ${JSON.stringify(mcpStart.argv)}`)
    }

    await client.send(requests.endSessionRequest(sessionId))
    sessionId = null
    await waitForNoPiProcesses(client, 20_000, options.pollMs)
    await assertFileExists(fakePiLog, "fake Pi log missing after successful drill")
    log("passed", {
      agentId: spawned.id,
      providerRunId: runWithResume.id,
      providerSessionId: runWithResume.provider_session_id,
      prompts: promptTexts.length,
    })
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client && sessionId) {
      await client.send(requests.endSessionRequest(sessionId)).catch(() => {})
    }
    await client?.close?.().catch(() => {})
    if (kernel) {
      await writeFile(path.join(rootDir, "kernel.stdout.log"), kernel.logs?.stdout ?? "", "utf8").catch(() => {})
      await writeFile(path.join(rootDir, "kernel.stderr.log"), kernel.logs?.stderr ?? "", "utf8").catch(() => {})
    }
    await stopChild(kernel)
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      preserveOnSuccess: false,
      failure,
      log,
      metadata: {
        drill: "live-pi-provider",
        kernelUrl,
        workspace,
        historyDir,
        fakePi,
        fakePiAuth,
        fakePiLog,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      },
    })
    if (passed && !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    }
  }
}

main().catch((error) => {
  console.error(error?.stack ?? error?.message ?? String(error))
  process.exit(1)
})

#!/usr/bin/env node
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import { mkdir, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false, skipBuild: process.env.CHARIOX_TUI_QUEUE_SKIP_BUILD === "1" }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--skip-build") options.skipBuild = true
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-queued-prompt-tui-e2e-drill.mjs [--skip-build] [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 52000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
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

async function buildRuntime(options) {
  if (options.skipBuild) {
    return path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel")
  }
  const kernel = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "chariox-kernel"])
  if (kernel.code !== 0) throw new Error(`kernel build failed\n${kernel.stdout}\n${kernel.stderr}`)
  const cli = await run("pnpm", ["--filter", "@chariox/cli", "run", "build"])
  if (cli.code !== 0) throw new Error(`cli build failed\n${cli.stdout}\n${cli.stderr}`)
  return path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel")
}

async function waitForKernel(LocalIpcClient, listSessionsRequest, kernelUrl) {
  const deadline = Date.now() + 20_000
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
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      socket.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
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

function unwrap(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key]) return resp[key]
  }
  return resp
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(5_000).then(() => false),
  ])
  if (!exited && child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForSnapshot(automation, predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send("snapshot")
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

function entriesForAgent(snapshot, agentId) {
  const paneEntries = snapshot.agentPanes && typeof snapshot.agentPanes === "object" && Array.isArray(snapshot.agentPanes[agentId])
    ? snapshot.agentPanes[agentId]
    : []
  if (paneEntries.length > 0) return paneEntries
  return Array.isArray(snapshot.transcript?.entries) ? snapshot.transcript.entries : []
}

function transcriptText(snapshot, agentId) {
  return entriesForAgent(snapshot, agentId).map((entry) => entry.text ?? "").join("\n")
}

function countOccurrences(text, needle) {
  if (!needle) return 0
  let count = 0
  let index = 0
  while ((index = text.indexOf(needle, index)) !== -1) {
    count += 1
    index += needle.length
  }
  return count
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, "target", "live-queued-prompt-tui-e2e-drill", `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, "workspace")
  const home = path.join(rootDir, "home")
  const configRoot = path.join(rootDir, "config")
  const stateRoot = path.join(rootDir, "state")
  const automationSocket = path.join(os.tmpdir(), `chariox-queued-prompt-tui-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configRoot,
    XDG_STATE_HOME: stateRoot,
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_DAEMON_ID: `queued-prompt-tui-e2e-${process.pid}-${Date.now()}`,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, "history-jsonl"),
    CHARIOX_TEST_TUI: "1",
  }

  let daemon = null
  let cli = null
  let cliStdout = ""
  let cliStderr = ""
  let automation = null
  let client = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(path.join(configRoot, "chariox"), { recursive: true })
    await mkdir(stateRoot, { recursive: true })
    await writeFile(path.join(configRoot, "chariox", "config.toml"), "version = 1\n", "utf8")

    const kernelBinary = await buildRuntime(options)
    const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
    const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ["ignore", "ignore", "inherit"] })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)

    const created = unwrap(
      await client.send(requests.createSessionRequest(workspace, workspace, "queued-prompt-tui-e2e")),
      "SessionCreated",
    )
    const session = created.session
    const agent = created.agent
    const controlAttachment = unwrap(
      await client.send(requests.attachToSessionRequest(session.id, `queued-prompt-tui-control-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const launched = unwrap(
      await client.send({
        LaunchProviderRun: {
          session_id: session.id,
          agent_id: agent.id,
          adapter_key: "dev-stub",
          provider: "slow-structured",
          account_profile: "default",
          model: "default",
          variant: "low",
          structured_endpoint: null,
          provider_session_id: null,
          native_tui: false,
        },
      }),
      "ProviderRunLaunched",
      "ProviderRunLaunchAccepted",
    )
    const providerRun = launched.provider_run

    const cliArgs = [
      "-q",
      "/dev/null",
      "env",
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      "bun",
      path.join(repoRoot, "apps/cli/dist/index.js"),
      "--kernel-url", kernelUrl,
      "--automation-socket", automationSocket,
      "--session", session.id,
      "--workspace", workspace,
      "--worktree", workspace,
      "--provider", "slow-structured",
      "--model", "default",
      "--client-id", `queued-prompt-tui-e2e-${process.pid}`,
    ]
    cli = spawn("script", cliArgs, { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
    cli.stdout.on("data", (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on("data", (chunk) => { cliStderr += chunk.toString() })
    const cliStartupFailure = new Promise((resolve) => {
      cli.once("error", (error) => resolve(error))
      cli.once("exit", (code, signal) => {
        if (code !== 0) resolve(new Error(`CLI exited before automation socket was ready: code=${code} signal=${signal ?? "none"}`))
      })
    })
    const startupFailure = await Promise.race([
      waitForSocket(automationSocket).then(() => null),
      cliStartupFailure,
    ])
    if (startupFailure) {
      throw new Error(`${startupFailure.message}\n--- cli stdout ---\n${cliStdout.slice(-4000)}\n--- cli stderr ---\n${cliStderr.slice(-4000)}`)
    }
    automation = createAutomationClient(automationSocket)
    await automation.send("ping")

    const firstPrompt = `TUI_QUEUE_FIRST_${process.pid}_${Date.now()} As a test, do 10 harmless tool calls, with assistant message after each call: assistant: 1 ... assistant: 10, then Done.`
    const queuedPrompt = `TUI_QUEUE_PENDING_${process.pid}_${Date.now()} Testing queued prompts. Acknowledge reception.`
    const editedQueuedPrompt = `${queuedPrompt} Edited before delivery.`
    const ackText = `TUI_QUEUE_ACK_${process.pid}_${Date.now()} Received this edited queued message.`

    const firstSubmit = unwrap(
      await client.send(requests.submitPromptRequest(session.id, controlAttachment.id, agent.id, firstPrompt, [])),
      "PromptSubmitted",
    )
    assert.equal(Object.keys(firstSubmit.outcome ?? {})[0], "Started")
    await waitForSnapshot(
      automation,
      (snapshot) => transcriptText(snapshot, agent.id).includes(firstPrompt),
      "first prompt visible in TUI transcript",
    )
    const queuedSubmit = unwrap(
      await client.send(requests.submitPromptRequest(session.id, controlAttachment.id, agent.id, queuedPrompt, [])),
      "PromptSubmitted",
    )
    assert.equal(Object.keys(queuedSubmit.outcome ?? {})[0], "Queued")
    const pending = await waitForSnapshot(
      automation,
      (snapshot) => {
        const items = snapshot.queuedPromptStrips?.[agent.id]?.items ?? []
        return items.some((item) => item.prompt === queuedPrompt) && !transcriptText(snapshot, agent.id).includes(queuedPrompt)
      },
      "queued prompt visible only in TUI pending strip",
    )
    const pendingPromptId = pending.queuedPromptStrips[agent.id].items.find((item) => item.prompt === queuedPrompt).promptId
    assert.match(pendingPromptId, /^pending-prompt-/)

    const updated = unwrap(
      await client.send(requests.updateQueuedPromptRequest(session.id, controlAttachment.id, agent.id, pendingPromptId, editedQueuedPrompt)),
      "QueuedPromptUpdated",
    )
    assert.equal(updated.prompt.id, pendingPromptId)
    await waitForSnapshot(
      automation,
      (snapshot) => {
        const items = snapshot.queuedPromptStrips?.[agent.id]?.items ?? []
        return items.some((item) => item.prompt === editedQueuedPrompt) && !transcriptText(snapshot, agent.id).includes(editedQueuedPrompt)
      },
      "edited queued prompt visible only in TUI pending strip",
    )

    for (let index = 1; index <= 10; index += 1) {
      await client.send(requests.appendNativeProviderOutputRequest(
        session.id,
        controlAttachment.id,
        providerRun.id,
        "provider_output",
        `assistant:${index}\n`,
        `queued-prompt-tui-first-${index}`,
      ))
    }
    await client.send(requests.appendNativeProviderOutputRequest(
      session.id,
      controlAttachment.id,
      providerRun.id,
      "provider_output",
      "Done.\n",
      "queued-prompt-tui-first-done",
    ))
    await waitForSnapshot(
      automation,
      (snapshot) => transcriptText(snapshot, agent.id).includes("assistant:10") && transcriptText(snapshot, agent.id).includes("Done."),
      "first turn output visible in TUI transcript",
    )
    await client.send(requests.completePromptRequest(session.id))
    await waitForSnapshot(
      automation,
      (snapshot) => transcriptText(snapshot, agent.id).includes(editedQueuedPrompt)
        && (snapshot.queuedPromptStrips?.[agent.id]?.items ?? []).length === 0,
      "queued prompt promoted in TUI transcript",
    )
    await client.send(requests.appendNativeProviderOutputRequest(
      session.id,
      controlAttachment.id,
      providerRun.id,
      "provider_output",
      `${ackText}\n`,
      "queued-prompt-tui-ack",
    ))
    await waitForSnapshot(
      automation,
      (snapshot) => transcriptText(snapshot, agent.id).includes(ackText),
      "queued prompt response visible in TUI transcript",
    )
    await client.send(requests.completePromptRequest(session.id))
    const finalSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => transcriptText(snapshot, agent.id).includes(ackText),
      "final TUI queued prompt transcript",
    )
    const finalText = transcriptText(finalSnapshot, agent.id)
    assert.equal(countOccurrences(finalText, firstPrompt), 1, "first prompt should appear exactly once")
    assert.equal(countOccurrences(finalText, queuedPrompt), 1, "queued prompt base text should appear once as part of edited prompt")
    assert.equal(countOccurrences(finalText, editedQueuedPrompt), 1, "edited queued prompt should appear exactly once")
    assert.equal(countOccurrences(finalText, ackText), 1, "queued prompt response should appear exactly once")
    assert.ok(finalText.indexOf("assistant:10") < finalText.indexOf(editedQueuedPrompt), "first turn output must precede queued prompt")
    assert.ok(finalText.indexOf(editedQueuedPrompt) < finalText.indexOf(ackText), "queued response must follow promoted prompt")

    console.log(JSON.stringify({
      drill: "queued-prompt-tui-e2e",
      rootDir,
      sessionId: session.id,
      agentId: agent.id,
      providerRunId: providerRun.id,
      pendingPromptId,
      transcriptEntryCount: entriesForAgent(finalSnapshot, agent.id).length,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    automation?.close()
    await client?.close?.().catch(() => {})
    await stopChild(cli)
    await stopChild(daemon)
    if (!succeeded && options.keepArtifactsOnFailure) {
      await mkdir(rootDir, { recursive: true }).catch(() => {})
      await writeFile(path.join(rootDir, "cli-stdout.log"), cliStdout, "utf8").catch(() => {})
      await writeFile(path.join(rootDir, "cli-stderr.log"), cliStderr, "utf8").catch(() => {})
    }
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: "queued-prompt-tui-e2e",
        kernelUrl,
        workspace,
        automationSocket,
      },
      log: (name, details) => console.log(`[queued-prompt-tui-e2e] ${name}`, JSON.stringify(details)),
    })
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})

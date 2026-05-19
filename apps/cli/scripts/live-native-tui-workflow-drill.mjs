import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowEndpointRequest,
  createWorkflowRequest,
  endSessionRequest,
  getSessionStateRequest,
  listAgentsRequest,
  pumpTerminalOutputRequest,
  setWorkflowFlushContextRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowRunOutputSchemaRequest,
  invokeWorkflowEndpointRequest,
  updateWorkflowNodeInstructionsRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const marker = `NTWF_${process.pid.toString(36)}_${Date.now().toString(36)}`
const hiddenMarker = "ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS"

function parseArgs(argv) {
  const options = {
    providers: ["opencode", "codex"],
    keepArtifactsOnFailure: false,
    pollLimit: 420,
    pollIntervalMs: 1_000,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    if (arg === "--providers") options.providers = argv[++index].split(",").map((value) => value.trim()).filter(Boolean)
    else if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--poll-limit") options.pollLimit = Number(argv[++index])
    else if (arg === "--poll-interval-ms") options.pollIntervalMs = Number(argv[++index])
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-native-tui-workflow-drill.mjs [--providers opencode,codex] [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.providers.length !== 2) throw new Error("native TUI workflow drill requires exactly two providers")
  for (const provider of options.providers) {
    if (provider !== "codex" && provider !== "opencode") throw new Error(`unsupported provider: ${provider}`)
  }
  return options
}

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function makePort() {
  return 53000 + Math.floor(Math.random() * 4000)
}

async function waitForDaemon(kernelUrl, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await client.send(createSessionRequest(workspace, worktree)), "SessionCreated").session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error("kernel did not become ready")
}

async function waitForFileMatch(file, pattern, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(file, "utf8").catch(() => "")
    const match = text.match(pattern)
    if (match) return { match, text }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${pattern} in ${file}\n${text.slice(-4000)}`)
}

async function screen(name, args) {
  await execFileAsync("screen", ["-S", name, ...args])
}

async function screenQuit(name) {
  await screen(name, ["-X", "quit"]).catch(() => {})
}

function startScreen(name, logDir, command, args, env) {
  return execFileAsync("screen", [
    "-dmS",
    name,
    "-L",
    command,
    ...args,
  ], { env, cwd: logDir })
}

async function waitForNamedAgents(client, sessionId, aliases) {
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
    const byAlias = new Map(agents.map((agent) => [agent.alias, agent]))
    if (aliases.every((alias) => byAlias.has(alias))) return aliases.map((alias) => byAlias.get(alias))
    await sleep(500)
  }
  throw new Error(`timed out waiting for native workflow agents ${aliases.join(", ")}`)
}

async function waitForLog(logFile, needle, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(logFile, "utf8").catch(() => "")
    if (text.includes(needle)) return text
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${needle} in ${logFile}\n${text.slice(-4000)}`)
}

function workflowOutput(summary, messageJson) {
  return [
    "```json",
    JSON.stringify({ summary, output: { message: messageJson } }, null, 2),
    "```",
  ].join("\n")
}

function nodePrompt(index) {
  if (index === 0) {
    return [
      "Read the endpoint prompt for the starting integer.",
      "Produce normal node-to-node workflow output for the downstream node.",
      "Set `output.message` to JSON with exactly one integer field: `value`.",
      "Use the integer from the endpoint prompt unchanged.",
      "Do not add any other fields.",
      "Your summary should be `sent 1842`.",
      workflowOutput("sent 1842", JSON.stringify({ value: 1842 })),
    ].join("\n\n")
  }
  return [
    "Read the upstream handoff payload for this workflow turn.",
    "Extract `output.message` JSON from the previous node.",
    "Read its integer field `value`.",
    "Add 1 to that integer.",
    "This node is the final workflow node. Generate final workflow run output JSON with exactly one integer field: `value` set to the incremented integer.",
    "Do not generate normal node-to-node output for this final result.",
    "Use the runtime MCP tool for final workflow run output submission and then finish the turn.",
    "If the runtime MCP tool is unavailable, emit the final fenced workflow JSON block with `output.message` set to the final result JSON.",
    workflowOutput("received 1842, completed 1843", JSON.stringify({ value: 1843 })),
    "Your summary should be `received 1842, completed 1843`.",
  ].join("\n\n")
}

async function ensureSchemaFile(root) {
  const schemaPath = path.join(root, "value-schema.json")
  await writeFile(schemaPath, JSON.stringify({
    $schema: "https://json-schema.org/draft/2020-12/schema",
    type: "object",
    required: ["value"],
    properties: { value: { type: "integer" } },
    additionalProperties: false,
  }, null, 2))
  return schemaPath
}

function nativeArgs(provider, { sessionId, kernelUrl, alias, sessionAlias, workspace, worktree }) {
  const args = [
    cliPath,
    provider,
    ...(sessionId ? [sessionId] : []),
    "--kernel-url",
    kernelUrl,
    "--agent-alias",
    alias,
    "--workspace",
    workspace,
    "--worktree",
    worktree,
    "--permissions",
    "yolo",
  ]
  if (!sessionId) args.push("--alias", sessionAlias)
  if (provider === "codex") args.push("--model", "gpt-5.4-mini", "--effort", "high")
  return args
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const root = path.join("/tmp", `arb-native-workflow-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const sessionAlias = `native-workflow-${marker}`
  const aliases = options.providers.map((provider, index) => `${provider === "codex" ? "cdx" : "oc"}-wf-${index + 1}`)
  const logs = options.providers.map((provider, index) => ({
    provider,
    alias: aliases[index],
    screen: `arroba-native-workflow-${provider}-${index + 1}-${process.pid}`,
    dir: path.join(root, `${provider}-${index + 1}-screen`),
    native: path.join(root, `${provider}-${index + 1}-screen`, "screenlog.0"),
    proxy: path.join(root, `${provider}-${index + 1}.proxy.log`),
  }))
  let daemon = null
  let client = null
  let sessionId = null
  let completed = false
  try {
    await mkdir(root, { recursive: true })
    for (const log of logs) await mkdir(log.dir, { recursive: true })
    const schemaPath = await ensureSchemaFile(root)
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-workflow-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(logs[0].screen, logs[0].dir, "bun", nativeArgs(options.providers[0], {
      kernelUrl,
      alias: aliases[0],
      sessionAlias,
      workspace,
      worktree,
    }), {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: options.providers[0] === "codex" ? "1" : undefined,
      ARROBA_CODEX_NATIVE_DEBUG_FILE: options.providers[0] === "codex" ? logs[0].proxy : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG: options.providers[0] === "opencode" ? "1" : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: options.providers[0] === "opencode" ? logs[0].proxy : undefined,
    })
    sessionId = (await waitForFileMatch(logs[0].native, /arroba session:\s+([^\s(]+)/)).match[1]

    for (let index = 1; index < options.providers.length; index += 1) {
      await startScreen(logs[index].screen, logs[index].dir, "bun", nativeArgs(options.providers[index], {
        sessionId,
        kernelUrl,
        alias: aliases[index],
        sessionAlias,
        workspace,
        worktree,
      }), {
        ...process.env,
        ARROBA_CODEX_NATIVE_DEBUG: options.providers[index] === "codex" ? "1" : undefined,
        ARROBA_CODEX_NATIVE_DEBUG_FILE: options.providers[index] === "codex" ? logs[index].proxy : undefined,
        ARROBA_OPENCODE_NATIVE_DEBUG: options.providers[index] === "opencode" ? "1" : undefined,
        ARROBA_OPENCODE_NATIVE_DEBUG_FILE: options.providers[index] === "opencode" ? logs[index].proxy : undefined,
      })
    }

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-workflow-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agents = await waitForNamedAgents(client, sessionId, aliases)
    for (const log of logs) {
      await waitForLog(log.proxy, log.provider === "codex" ? "provider_run_bound" : "proxy_listening")
    }

    const workflow = unwrap(await client.send(createWorkflowRequest(sessionId, `native-tui-final-output-${marker}`)), "WorkflowCreated").workflow
    await client.send(setWorkflowFlushContextRequest(sessionId, workflow.id, false))
    const nodeIds = []
    for (let index = 0; index < agents.length; index += 1) {
      const node = unwrap(await client.send(addWorkflowNodeRequest(sessionId, workflow.id, agents[index].id)), "WorkflowNodeAdded").node
      nodeIds.push(node.id)
      await client.send(updateWorkflowNodeInstructionsRequest(sessionId, workflow.id, node.id, nodePrompt(index)))
      if (index > 0) {
        await client.send(addWorkflowEdgeRequest(sessionId, workflow.id, nodeIds[index - 1], nodeIds[index]))
      }
    }
    await client.send(setWorkflowRunOutputSchemaRequest(sessionId, workflow.id, schemaPath))
    await client.send(setWorkflowNodeCanCompleteRunRequest(sessionId, workflow.id, nodeIds[1], true))
    const endpoint = unwrap(
      await client.send(createWorkflowEndpointRequest(sessionId, workflow.id, nodeIds[0], "start")),
      "WorkflowEndpointCreated",
    ).endpoint
    const workflowRun = unwrap(
      await client.send(invokeWorkflowEndpointRequest(sessionId, workflow.id, endpoint.id, "Start the workflow with integer 1842. The workflow should return the incremented final result.")),
      "WorkflowRunInvoked",
    ).workflow_run

    let completedRun = null
    for (let attempt = 0; attempt < options.pollLimit; attempt += 1) {
      await sleep(options.pollIntervalMs)
      await client.send(pumpTerminalOutputRequest(sessionId, attachment.id)).catch(() => {})
      const stateResp = await client.send(getSessionStateRequest(sessionId))
      const state = (stateResp.SessionState ?? stateResp.SessionStateLoaded).session
      const run = (state.workflow_runs || []).find((entry) => entry.id === workflowRun.id)
      if (run && ["Completed", "Failed", "Stopped"].includes(run.status)) {
        completedRun = run
        break
      }
    }
    if (!completedRun) throw new Error(`workflow run ${workflowRun.id} did not finish`)
    if (completedRun.status !== "Completed") {
      throw new Error(`workflow run ${workflowRun.id} ended with ${completedRun.status}`)
    }
    const expectedFinalOutput = JSON.stringify({ value: 1843 })
    if (completedRun.final_output?.message !== expectedFinalOutput) {
      throw new Error(`workflow final output mismatch: expected ${expectedFinalOutput}, got ${completedRun.final_output?.message}`)
    }
    for (const log of logs) {
      await waitForLog(log.proxy, "hidden_instructions_forwarded")
      const nativeLog = await readFile(log.native, "utf8").catch(() => "")
      const leakedWorkflowInjection = [
        hiddenMarker,
        "list_extensions",
        "You are an agent participating in an Arroba workflow turn",
        "System node-level prompt:",
        "Node instruction reference (daemon-managed)",
      ].find((needle) => nativeLog.includes(needle))
      if (leakedWorkflowInjection) {
        throw new Error(`${log.alias} native TUI displayed hidden Arroba workflow prompt injection: ${leakedWorkflowInjection}`)
      }
    }

    console.log(JSON.stringify({
      status: "ok",
      mode: "native-tui-workflow",
      sessionId,
      workflowId: workflow.id,
      workflowRunId: workflowRun.id,
      providers: options.providers,
      agentAliases: aliases,
      finalOutput: completedRun.final_output,
      promptInjection: "hidden block forwarded to provider server and redacted from native TUIs",
      logs: Object.fromEntries(logs.map((log) => [log.alias, { native: log.native, proxy: log.proxy }])),
    }, null, 2))
    completed = true
  } finally {
    if (client) await client.close().catch(() => {})
    for (const log of logs) await screenQuit(log.screen)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    if (process.env.ARROBA_KEEP_NATIVE_TUI_WORKFLOW_ARTIFACTS === "1" || (options.keepArtifactsOnFailure && !completed)) {
      console.log(JSON.stringify({ artifactsKept: root }))
    } else {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})

import path from "node:path"
import { mkdir, rm, writeFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../../dist/ipc.js"
import {
  getProviderRunRequest,
  installMcpServerRequest,
  installSkillRequest,
} from "../../dist/ipc-requests.js"

export async function installNativeDrillCapabilities({
  homeClient,
  workerKernelUrl,
  provider,
  scenarioRoot,
  workspace,
  options,
  markers,
}) {
  if (!options.includeMcpSkills) return null
  if (options.hetznerWorker) {
    throw new Error("--include-mcp-skills is not implemented for --hetzner-worker yet; use same-host standard remote or home-managed slice")
  }
  const normalizedProvider = provider.replaceAll(/[^a-z0-9_-]/gi, "-").toLowerCase()
  const suffix = `${process.pid}-${Date.now()}`
  const mcpName = `native-${normalizedProvider}-${suffix}-node`
  const skillName = `native-${normalizedProvider}-${suffix}-skill`
  const mcpServerPath = await createNativeDrillMcpServer(workspace, mcpName)
  const mcpConfig = nativeDrillMcpConfig(mcpName, mcpServerPath)
  const skillSource = await createNativeDrillSkill(
    path.join(scenarioRoot, "skill-source"),
    skillName,
    markers.nativeSkill,
    markers.arrobaSkill,
  )
  await homeClient.send(installMcpServerRequest(workspace, mcpConfig))
  const installedSkill = unwrapVariant(
    await homeClient.send(installSkillRequest(workspace, skillSource)),
    "SkillInstalled",
  ).skill

  if (options.standardHomeWorker && workerKernelUrl) {
    const workerClient = new LocalIpcClient(workerKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await workerClient.send(installMcpServerRequest(workspace, mcpConfig))
    } finally {
      await workerClient.close().catch(() => {})
    }
  }

  return {
    mcpName,
    mcpServerPath,
    skillName: installedSkill?.name ?? skillName,
  }
}

export async function cleanupNativeDrillCapabilities(workspace, nativeCapabilities) {
  if (!nativeCapabilities) return
  await rm(path.join(workspace, ".arroba", "skills", nativeCapabilities.skillName), {
    recursive: true,
    force: true,
  }).catch(() => {})
  await rm(path.join(workspace, ".arroba", "mcps", `${nativeCapabilities.mcpName}.json`), {
    force: true,
  }).catch(() => {})
  await rm(nativeCapabilities.mcpServerPath, { force: true }).catch(() => {})
}

export async function waitForProviderRunMcpGrant(client, providerRunId, mcpName, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  while (Date.now() < deadline) {
    const run = unwrapVariant(
      await client.send(getProviderRunRequest(providerRunId)),
      "ProviderRun",
    ).provider_run
    lastRun = run
    const mcps = run?.mcp_servers ?? []
    if (mcps.some((mcp) => mcp.name === mcpName)) return run
    await sleep(500)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId} MCP grant ${mcpName}; last=${JSON.stringify(lastRun)}`)
}

async function createNativeDrillMcpServer(workspace, name) {
  const scriptDir = path.join(workspace, ".arroba", "native-tui-drill")
  const scriptPath = path.join(scriptDir, `${name}.mjs`)
  await mkdir(scriptDir, { recursive: true })
  await writeFile(scriptPath, [
    "let buffer = Buffer.alloc(0)",
    "function write(message) {",
    "  const body = Buffer.from(JSON.stringify(message), 'utf8')",
    "  process.stdout.write(`Content-Length: ${body.length}\\r\\n\\r\\n`)",
    "  process.stdout.write(body)",
    "}",
    "function handle(message) {",
    "  const { id, method, params } = message",
    "  if (method === 'notifications/initialized') return",
    "  if (method === 'initialize') {",
    "    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'arroba-native-tui-drill', version: '1.0.0' } } })",
    "    return",
    "  }",
    "  if (method === 'tools/list') {",
    "    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'echo_marker', description: 'Echoes a marker for Arroba native TUI MCP drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })",
    "    return",
    "  }",
    "  if (method === 'tools/call' && params?.name === 'echo_marker') {",
    "    write({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `ECHO:${params?.arguments?.marker ?? ''}` }] } })",
    "    return",
    "  }",
    "  write({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })",
    "}",
    "process.stdin.on('data', (chunk) => {",
    "  buffer = Buffer.concat([buffer, chunk])",
    "  while (true) {",
    "    const newline = buffer.indexOf('\\n')",
    "    if (newline >= 0) {",
    "      const line = buffer.subarray(0, newline).toString('utf8').trim()",
    "      buffer = buffer.subarray(newline + 1)",
    "      if (line) handle(JSON.parse(line))",
    "      continue",
    "    }",
    "    const headerEnd = buffer.indexOf('\\r\\n\\r\\n')",
    "    if (headerEnd < 0) return",
    "    const header = buffer.subarray(0, headerEnd).toString('utf8')",
    "    const match = /^content-length:\\s*(\\d+)$/im.exec(header)",
    "    if (!match) throw new Error(`missing Content-Length: ${header}`)",
    "    const length = Number(match[1])",
    "    const bodyStart = headerEnd + 4",
    "    const frameEnd = bodyStart + length",
    "    if (buffer.length < frameEnd) return",
    "    const message = JSON.parse(buffer.subarray(bodyStart, frameEnd).toString('utf8'))",
    "    buffer = buffer.subarray(frameEnd)",
    "    handle(message)",
    "  }",
    "})",
  ].join("\n"), "utf8")
  return scriptPath
}

function nativeDrillMcpConfig(name, command) {
  return {
    name,
    transport: {
      type: "stdio",
      command: "node",
      args: [command],
    },
    enabled: true,
    required: true,
    tools: {},
  }
}

async function createNativeDrillSkill(sourceRoot, name, nativeMarker, arrobaMarker) {
  const skillDir = path.join(sourceRoot, name)
  await mkdir(skillDir, { recursive: true })
  await writeFile(path.join(skillDir, "SKILL.md"), [
    "---",
    `name: ${name}`,
    `description: Native TUI drill skill for ${name}.`,
    "short-description: Native TUI drill",
    "---",
    `If the prompt asks for the native skill marker, reply with exactly ${nativeMarker} and nothing else.`,
    `If the prompt asks for the Arroba skill marker, reply with exactly ${arrobaMarker} and nothing else.`,
    "",
  ].join("\n"))
  return skillDir
}

function unwrapVariant(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

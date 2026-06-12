import { spawn } from 'node:child_process'
import { access, mkdir, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { runNodeDrillChild } from './lib/drill-child-process.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await (await import('node:fs/promises')).readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await (await import('node:fs/promises')).writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

let LocalIpcClient
let createSessionRequest
let endSessionRequest
let listRemoteMachinesRequest
let installMcpServerRequest

const DEFAULT_SCENARIO = 'validated-increment-chain'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_POLL_LIMIT = 120
const DEFAULT_POLL_INTERVAL_MS = 2000

function parseArgs(argv) {
  const options = {
    scenario: DEFAULT_SCENARIO,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    pollLimit: DEFAULT_POLL_LIMIT,
    pollIntervalMs: DEFAULT_POLL_INTERVAL_MS,
    noEarlyPass: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--scenario') options.scenario = argv[++i]
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--poll-limit') options.pollLimit = Number(argv[++i])
    else if (arg === '--poll-interval-ms') options.pollIntervalMs = Number(argv[++i])
    else if (arg === '--no-early-pass') options.noEarlyPass = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function makePorts() {
  const base = 54000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    homeKernelPort: base + 1000,
    workerKernelPort: base + 1001,
    homeMcpPort: base + 2000,
    workerMcpPort: base + 2001,
    homeOpenCodePort: base + 3000,
    workerOpenCodePort: base + 3001,
    homeCodexPort: base + 3002,
    workerCodexPort: base + 3003,
  }
}

function daemonEnv({
  ports,
  rootDir,
  relayToken,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  acceptRemoteLeases,
  kernelPort,
  mcpPort,
  opencodePort,
  codexPort,
  socketName,
}) {
  return {
    ...process.env,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    ARROBA_DAEMON_SOCKET: path.join(rootDir, socketName),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonId}-history`),
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonId}-xdg-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonId}-xdg-state`),
    XDG_CACHE_HOME: path.join(rootDir, `${daemonId}-xdg-cache`),
  }
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'inherit'] })
}

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernelClient() {
  const result = await runCommand('pnpm', ['--workspace-root', 'run', 'build:kernel-client'])
  if (result.code !== 0) {
    throw new Error(`kernel client build failed\n${result.stdout}\n${result.stderr}`)
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp


async function createWorkflowEchoMcp(rootDir) {
  const mcpPath = path.join(rootDir, 'workflow-echo-mcp.mjs')
  await mkdir(rootDir, { recursive: true })
  await writeFile(mcpPath, [
    "let buffer = Buffer.alloc(0)",
    "function write(message) {",
    "  const body = JSON.stringify(message)",
    "  process.stdout.write(`${body}\\n`)",
    "}",
    "function handle(message) {",
    "  const { id, method, params } = message",
    "  if (method === 'notifications/initialized') return",
    "  if (method === 'initialize') {",
    "    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'arroba-workflow-echo', version: '1.0.0' } } })",
    "    return",
    "  }",
    "  if (method === 'tools/list') {",
    "    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'echo_marker', description: 'Echoes a marker for Arroba workflow MCP drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })",
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
  ].join('\n'), 'utf8')
  return mcpPath
}

function workflowEchoMcpConfig(mcpPath) {
  return {
    name: 'workflow_echo',
    transport: { type: 'stdio', command: 'node', args: [mcpPath], env: {}, env_vars: [] },
    enabled: true,
    required: false,
    startup_timeout_sec: 5,
    tool_timeout_sec: 10,
  }
}

async function installWorkerWorkflowEchoMcp(workerKernelUrl, workspace) {
  const workerClient = new LocalIpcClient(workerKernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    const mcpPath = await createWorkflowEchoMcp(path.join(workspace, 'tmp', 'live-drills'))
    await workerClient.send(installMcpServerRequest(workspace, workflowEchoMcpConfig(mcpPath)))
  } finally {
    await workerClient.close().catch(() => {})
  }
}

async function waitForTcpPort(port, host = '127.0.0.1', timeoutMs = 15_000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const connected = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (connected) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForLocalDaemon(kernelUrl) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      const session = unwrap(await probe.send(createSessionRequest(repoRoot, repoRoot)), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error('local daemon did not become ready')
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForRemoteMachine(client, machineRef) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

async function waitForRemoteKernel(client, machineRef, providers) {
  let last = []
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const response = unwrapVariant(
      await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
      'RemoteMachineKernelsListed',
    )
    last = response.kernels || []
    const kernel = last.find((candidate) => candidate.accepting_remote_leases
      && providers.every((provider) => (candidate.available_providers || []).includes(provider)))
    if (kernel) return kernel
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not advertise providers ${providers.join(',')}; last=${JSON.stringify(last)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log('Usage: node apps/cli/scripts/live-remote-workflow-runtime-drill.mjs [--scenario validated-increment-chain] [--providers opencode,codex] [--model MODEL] [--provider-model PROVIDER=MODEL] [--no-early-pass]')
    return
  }
  if (options.providers.length < 1) {
    throw new Error('remote workflow runtime drill requires at least one provider')
  }

  const ports = makePorts()
  const rootDir = path.join(os.tmpdir(), `arroba-remote-workflow-${process.pid}-${Date.now()}`)
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-workflow-runtime-drill')
  await mkdir(rootDir, { recursive: true })
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  await buildKernelClient()
  ;({ LocalIpcClient, requests: {
    createSessionRequest,
    endSessionRequest,
    listRemoteMachinesRequest,
    installMcpServerRequest,
  } } = await loadCliModules(cliRuntimeDir))

  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const relayBinary = await resolveBinary(
    path.join(repoRoot, 'apps/relay/target/debug/arroba-relay'),
    path.join(repoRoot, 'apps/relay/Cargo.toml'),
    'arroba-relay',
  )
  const daemonBinary = await resolveBinary(
    path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    'arroba-kernel',
  )
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: '127.0.0.1',
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_TOKEN: relayToken,
  }
  const workerMachineId = `workflow-machine-worker-${process.pid}`
  const workerMachineAlias = `workflow-builder-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let localClient = null

  try {
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    await waitForTcpPort(ports.relayPort)
    homeChild = spawnProcess(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: `workflow-home-${process.pid}-${Date.now()}`,
        daemonAlias: 'home',
        machineId: `workflow-machine-home-${process.pid}`,
        machineAlias: `workflow-home-machine-${process.pid}`,
        acceptRemoteLeases: false,
        kernelPort: ports.homeKernelPort,
        mcpPort: ports.homeMcpPort,
        opencodePort: ports.homeOpenCodePort,
        codexPort: ports.homeCodexPort,
        socketName: 'home.sock',
      }),
    })
    workerChild = spawnProcess(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: `workflow-worker-${process.pid}-${Date.now()}`,
        daemonAlias: 'worker',
        machineId: workerMachineId,
        machineAlias: workerMachineAlias,
        acceptRemoteLeases: true,
        kernelPort: ports.workerKernelPort,
        mcpPort: ports.workerMcpPort,
        opencodePort: ports.workerOpenCodePort,
        codexPort: ports.workerCodexPort,
        socketName: 'worker.sock',
      }),
    })

    await waitForLocalDaemon(homeKernelUrl)
    await waitForLocalDaemon(`ws://127.0.0.1:${ports.workerKernelPort}`)
    if (options.scenario === 'mcp-echo-workflow') {
      await installWorkerWorkflowEchoMcp(`ws://127.0.0.1:${ports.workerKernelPort}`, repoRoot)
    }
    await waitForRelayTarget(relayUrl, relayToken, 'home')
    await waitForRelayTarget(relayUrl, relayToken, 'worker')

    localClient = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForRemoteMachine(localClient, workerMachineId)
    const workerKernel = await waitForRemoteKernel(localClient, workerMachineId, Array.from(new Set(options.providers)))

    const workflowArgs = [
      path.join('apps', 'cli', 'scripts', 'live-workflow-runtime-drill.mjs'),
      '--scenario', options.scenario,
      '--relay-url', relayUrl,
      '--relay-token', relayToken,
      '--target-daemon-alias', 'home',
      '--machine-ref', workerMachineId,
      '--providers', options.providers.join(','),
      '--model', options.model,
      ...Object.entries(options.providerModels).flatMap(([provider, model]) => ['--provider-model', `${provider}=${model}`]),
      '--workspace', repoRoot,
      '--worktree', repoRoot,
      '--poll-limit', String(options.pollLimit),
      '--poll-interval-ms', String(options.pollIntervalMs),
    ]
    if (options.noEarlyPass) workflowArgs.push('--no-early-pass')
    const stdout = await runNodeDrillChild(workflowArgs, repoRoot, {
      label: 'remote workflow runtime drill',
    })

    const trimmed = stdout.trim()
    const lastJsonIndex = trimmed.lastIndexOf('\n{')
    const jsonText = lastJsonIndex >= 0 ? trimmed.slice(lastJsonIndex + 1) : trimmed
    const result = JSON.parse(jsonText)
    console.log(JSON.stringify({
      status: 'ok',
      relayUrl,
      homeKernelUrl,
      workerMachineId,
      workerMachineAlias,
      workerKernel: {
        kernelId: workerKernel.kernel_id,
        machineId: workerKernel.machine_id,
        providers: workerKernel.available_providers,
      },
      scenario: options.scenario,
      providers: options.providers,
      model: options.model,
      providerModels: options.providerModels,
      workflow: result,
    }, null, 2))
  } finally {
    if (localClient) await localClient.close().catch(() => {})
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()

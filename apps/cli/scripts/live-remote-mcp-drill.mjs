import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_PROVIDER = 'opencode'
const DEFAULT_MODEL = 'openai/gpt-5.2'
const DEFAULT_TIMEOUT_MS = 180_000

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: 'low',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-remote-mcp-drill.mjs [options]',
    '',
    'Runs a remote M7 MCP conformance drill with isolated relay/home/worker daemons:',
    '- uses separate HOME roots for home and worker kernels',
    '- verifies worker missing MCP fails fast',
    '- verifies worker global MCP hash mismatch fails fast',
    '- verifies worker project-local MCP override wins over mismatched global config',
    '- verifies missing stdio command fails fast',
    '- verifies missing env var fails fast',
    '',
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --effort low',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

function makePorts() {
  const base = 56000 + Math.floor(Math.random() * 1000)
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
  historyDir,
  homeDir,
}) {
  return {
    ...process.env,
    HOME: homeDir,
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
    ARROBA_SESSION_HISTORY_DIR: historyDir,
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

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

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error('home daemon did not become ready')
}

async function waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, targetDaemonAlias) {
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
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable`)
}

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

function spawnRemoteAgentRequest(sessionId, provider, alias, model, worktreeId, effort, machineRef) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort,
      worktree_id: worktreeId,
      machine_ref: machineRef,
    },
  }
}

async function writeMcp(root, config) {
  const dir = path.join(root, '.arroba', 'mcps')
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, `${config.name}.json`), `${JSON.stringify(config, null, 2)}\n`, 'utf8')
}

async function clearProjectMcps(workspace) {
  await rm(path.join(workspace, '.arroba', 'mcps'), { recursive: true, force: true })
}

function mcpConfig(name, command, extra = {}) {
  return {
    name,
    transport: {
      type: 'stdio',
      command,
      args: extra.args ?? [],
      env: extra.env ?? {},
      env_vars: extra.env_vars ?? [],
      cwd: extra.cwd ?? undefined,
    },
    enabled: true,
    required: false,
    tools: {},
  }
}

async function expectReject(label, fn, expectedText) {
  try {
    await fn()
  } catch (error) {
    const message = String(error?.message ?? error)
    if (!message.includes(expectedText)) {
      throw new Error(`${label} rejected with unexpected error. Expected ${expectedText}, got: ${message}`)
    }
    return { ok: true, error: message }
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const ports = makePorts()
  const rootDir = path.join(os.tmpdir(), `arroba-remote-mcp-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const homeHomeDir = path.join(rootDir, 'home-home')
  const workerHomeDir = path.join(rootDir, 'worker-home')
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-mcp-drill')
  await mkdir(workspace, { recursive: true })
  await mkdir(homeHomeDir, { recursive: true })
  await mkdir(workerHomeDir, { recursive: true })
  await rm(cliRuntimeDir, { recursive: true, force: true })
  await mkdir(cliRuntimeDir, { recursive: true })

  const { LocalIpcClient, requests } = await loadCliModules(cliRuntimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    grantAgentCapabilityRequest,
    listRemoteMachinesRequest,
  } = requests

  const relayToken = `remote-mcp-token-${process.pid}`
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
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const workerMachineId = `remote-mcp-machine-worker-${process.pid}`
  const workerMachineAlias = `remote-mcp-worker-${process.pid}`
  const homeDaemonId = `remote-mcp-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `remote-mcp-worker-${process.pid}-${Date.now()}`
  const homeHistoryDir = path.join(rootDir, `${homeDaemonId}-history`)
  const workerHistoryDir = path.join(rootDir, `${workerDaemonId}-history`)

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let client = null
  let succeeded = false
  try {
    relayChild = spawn(relayBinary, [], { cwd: repoRoot, env: relayEnv, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForTcpPort(ports.relayPort)
    homeChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: homeDaemonId,
        daemonAlias: 'home',
        machineId: `remote-mcp-machine-home-${process.pid}`,
        machineAlias: `remote-mcp-home-machine-${process.pid}`,
        acceptRemoteLeases: false,
        kernelPort: ports.homeKernelPort,
        mcpPort: ports.homeMcpPort,
        opencodePort: ports.homeOpenCodePort,
        codexPort: ports.homeCodexPort,
        socketName: 'home.sock',
        historyDir: homeHistoryDir,
        homeDir: homeHomeDir,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    workerChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: workerDaemonId,
        daemonAlias: 'worker',
        machineId: workerMachineId,
        machineAlias: workerMachineAlias,
        acceptRemoteLeases: true,
        kernelPort: ports.workerKernelPort,
        mcpPort: ports.workerMcpPort,
        opencodePort: ports.workerOpenCodePort,
        codexPort: ports.workerCodexPort,
        socketName: 'worker.sock',
        historyDir: workerHistoryDir,
        homeDir: workerHomeDir,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, homeKernelUrl, createSessionRequest, endSessionRequest, workspace)
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'worker')

    client = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForRemoteMachine(client, listRemoteMachinesRequest, workerMachineId)

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    await client.send(attachToSessionRequest(session.id, `remote-mcp-drill-${Date.now()}`))

    const matching = mcpConfig('remote-mcp-drill', 'node', { args: ['--version'] })
    const mismatch = mcpConfig('remote-mcp-drill', 'node', { args: ['--eval', 'console.log("mismatch")'] })
    const missingCommand = mcpConfig('remote-mcp-missing-command', 'arroba-definitely-missing-mcp-command')
    const missingEnv = mcpConfig('remote-mcp-missing-env', 'node', { env_vars: ['ARROBA_REMOTE_MCP_DRILL_REQUIRED_ENV'] })

    const scenarioResults = []

    await writeMcp(homeHomeDir, matching)
    const missingWorkerAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-missing-worker-agent',
      options.model,
      workspace,
      options.effort,
      workerMachineId,
    )), 'AgentSpawned').agent
    scenarioResults.push({
      scenario: 'worker_missing_mcp_fails_fast',
      ...(await expectReject(
        'worker missing MCP',
        () => client.send(grantAgentCapabilityRequest(workspace, missingWorkerAgent.id, 'mcp', 'remote-mcp-drill')),
        'missing on worker',
      )),
    })

    await writeMcp(workerHomeDir, mismatch)
    const mismatchAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-global-mismatch-agent',
      options.model,
      workspace,
      options.effort,
      workerMachineId,
    )), 'AgentSpawned').agent
    scenarioResults.push({
      scenario: 'worker_global_mismatch_fails_fast',
      ...(await expectReject(
        'worker global mismatch',
        () => client.send(grantAgentCapabilityRequest(workspace, mismatchAgent.id, 'mcp', 'remote-mcp-drill')),
        'definition mismatch',
      )),
    })

    await writeMcp(workspace, matching)
    const overrideAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-project-override-agent',
      options.model,
      workspace,
      options.effort,
      workerMachineId,
    )), 'AgentSpawned').agent
    const overrideGranted = unwrapVariant(await client.send(grantAgentCapabilityRequest(
      workspace,
      overrideAgent.id,
      'mcp',
      'remote-mcp-drill',
    )), 'AgentCapabilityGranted').agent
    scenarioResults.push({
      scenario: 'worker_project_local_override_passes',
      ok: true,
      agent: { id: overrideGranted.id, ref: overrideGranted.agent_ref, mcp_grants: overrideGranted.mcp_grants },
    })

    await clearProjectMcps(workspace)
    await writeMcp(homeHomeDir, missingCommand)
    await writeMcp(workerHomeDir, missingCommand)
    const missingCommandAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-missing-command-agent',
      options.model,
      workspace,
      options.effort,
      workerMachineId,
    )), 'AgentSpawned').agent
    scenarioResults.push({
      scenario: 'worker_missing_command_fails_fast',
      ...(await expectReject(
        'worker missing command',
        () => client.send(grantAgentCapabilityRequest(workspace, missingCommandAgent.id, 'mcp', 'remote-mcp-missing-command')),
        'missing command',
      )),
    })

    await writeMcp(homeHomeDir, missingEnv)
    await writeMcp(workerHomeDir, missingEnv)
    const missingEnvAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-missing-env-agent',
      options.model,
      workspace,
      options.effort,
      workerMachineId,
    )), 'AgentSpawned').agent
    scenarioResults.push({
      scenario: 'worker_missing_env_fails_fast',
      ...(await expectReject(
        'worker missing env',
        () => client.send(grantAgentCapabilityRequest(workspace, missingEnvAgent.id, 'mcp', 'remote-mcp-missing-env')),
        'missing environment variable',
      )),
    })

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'remote-mcp-conformance-drill',
      relayUrl,
      homeKernelUrl,
      workerMachineId,
      workerMachineAlias,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      homeHomeDir,
      workerHomeDir,
      workspace,
      scenarios: scenarioResults,
    }, null, 2))
    succeeded = true
  } finally {
    if (client) await client.close().catch(() => {})
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
      await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote MCP drill artifacts kept at ${rootDir}`)
      console.error(`remote MCP drill transient CLI modules kept at ${cliRuntimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

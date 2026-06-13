import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
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

const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function defaultModelForProvider(provider) {
  if (provider === 'opencode') return 'opencode/gpt-5.4'
  return 'gpt-5.4'
}

function makePorts() {
  const base = 59000 + Math.floor(Math.random() * 1000)
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
    ARROBA_SESSION_HISTORY_DIR: historyDir,
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonId}-xdg-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonId}-xdg-state`),
    XDG_CACHE_HOME: path.join(rootDir, `${daemonId}-xdg-cache`),
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

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log('Usage: node apps/cli/scripts/live-remote-popup-drill.mjs [--providers opencode,codex] [--model MODEL] [--provider-model PROVIDER=MODEL]')
    return
  }
  if (options.providers.length < 1) throw new Error('remote popup drill requires at least one provider')

  const runId = `${process.pid}-${Date.now()}`
  const ports = makePorts()
  const rootDir = path.join(os.tmpdir(), `arroba-remote-popup-${runId}`)
  const cliRuntimeDir = path.join(cliRoot, `.tmp-live-remote-popup-drill-${runId}`)
  await prepareDrillArtifacts(rootDir)
  await mkdir(cliRuntimeDir, { recursive: true })

  const { LocalIpcClient, requests } = await loadCliModules(cliRuntimeDir)
  const {
    createSessionRequest,
    endSessionRequest,
    listRemoteMachinesRequest,
    setUserConfigValueRequest,
  } = requests

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
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const workerKernelUrl = `ws://127.0.0.1:${ports.workerKernelPort}`
  const workerMachineId = `popup-machine-worker-${process.pid}`
  const homeDaemonId = `popup-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `popup-worker-${process.pid}-${Date.now()}`
  const homeHistoryDir = path.join(rootDir, `${homeDaemonId}-history`)
  const workerHistoryDir = path.join(rootDir, `${workerDaemonId}-history`)

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let localClient = null
  let succeeded = false
  let failure = null

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
        machineId: `popup-machine-home-${process.pid}`,
        machineAlias: `popup-home-machine-${process.pid}`,
        acceptRemoteLeases: false,
        kernelPort: ports.homeKernelPort,
        mcpPort: ports.homeMcpPort,
        opencodePort: ports.homeOpenCodePort,
        codexPort: ports.homeCodexPort,
        socketName: 'home.sock',
        historyDir: homeHistoryDir,
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
        machineAlias: `popup-worker-${process.pid}`,
        acceptRemoteLeases: true,
        kernelPort: ports.workerKernelPort,
        mcpPort: ports.workerMcpPort,
        opencodePort: ports.workerOpenCodePort,
        codexPort: ports.workerCodexPort,
        socketName: 'worker.sock',
        historyDir: workerHistoryDir,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, homeKernelUrl, createSessionRequest, endSessionRequest, repoRoot)
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'worker')

    localClient = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForRemoteMachine(localClient, listRemoteMachinesRequest, workerMachineId)

    const results = []
    for (const provider of options.providers) {
      const model = options.providerModels[provider]
        ?? (options.model === DEFAULT_MODEL ? defaultModelForProvider(provider) : options.model)
      const workerClient = new LocalIpcClient(workerKernelUrl, {
        kernelPingIntervalMs: 60_000,
        kernelMaxMissedPongs: 10,
      })
      try {
        await localClient.send(setUserConfigValueRequest('providers.workspace_live_sync', 'off'))
        await workerClient.send(setUserConfigValueRequest('providers.workspace_live_sync', 'off'))
      } finally {
        await workerClient.close().catch(() => {})
      }
      await runNodeDrillChild([
        path.join('apps', 'cli', 'scripts', 'live-popup-drill.mjs'),
        '--kernel', homeKernelUrl,
        '--no-spawn-daemon',
        '--machine-ref', workerMachineId,
        '--provider', provider,
        '--model', model,
        '--timeout-ms', String(options.timeoutMs),
        '--poll-ms', String(options.pollMs),
        ...(options.keepArtifactsOnFailure ? ['--keep-artifacts-on-failure'] : []),
      ], repoRoot, { label: 'remote popup drill' })
      results.push({ provider, model, status: 'passed' })
    }

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'remote-popup-live-drill',
      relayUrl,
      homeKernelUrl,
      workerMachineId,
      providers: options.providers,
      model: options.model,
      providerModels: options.providerModels,
      results,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (localClient) await localClient.close().catch(() => {})
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'remote-popup',
        providers: options.providers.join(','),
        model: options.model,
        providerModels: options.providerModels,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        relayUrl,
        homeKernelUrl,
        workerKernelUrl,
        workerMachineId,
        cliRuntimeDir,
      },
      log: (name, details) => console.log(`[remote-popup-drill] ${name}`, JSON.stringify(details)),
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote popup drill artifacts kept at ${rootDir}`)
      console.error(`remote popup drill transient CLI modules kept at ${cliRuntimeDir}`)
    }
  }
}

await main()

import { execFile, spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { runNodeDrillChild } from './lib/drill-child-process.mjs'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  assertHetznerArrobaBinaries,
  assertHetznerTcpPortAvailable,
  remoteEnvCommand,
  runHetznerCommand,
  shellQuote,
  sshArgs,
  stopHetznerProcessByEnv,
} from './lib/native-tui-remote-execution.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const execFileAsync = promisify(execFile)

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
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000
const HETZNER_MANAGED_WORKSPACE_LIVE_SYNC_UNSUPPORTED_REASON =
  'Hetzner managed Workspace Live Sync is unsupported because the worker runs Linux and managed mode needs selective write fencing, which is only implemented on macOS; use tracked mode on this worker or run the managed provider on a supported host'

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    full: false,
    mode: 'managed',
    managedTargetCount: 0,
    targetBranch: 'main',
    restartRelayBeforeSync: false,
    trackedTargetCount: 1,
    trackedBidirectional: false,
    hetznerWorker: false,
    hetznerHost: process.env.ARROBA_WORKSPACE_LIVE_SYNC_HETZNER_HOST ?? process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? 'root@195.201.123.115',
    hetznerKey: process.env.ARROBA_WORKSPACE_LIVE_SYNC_HETZNER_KEY ?? process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), '.ssh/arroba_hetzner_staging'),
    hetznerRepo: process.env.ARROBA_WORKSPACE_LIVE_SYNC_HETZNER_REPO ?? process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? '/tmp/arroba-native-remote-validate',
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
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
    else if (arg === '--full') options.full = true
    else if (arg === '--restart-relay-before-sync') options.restartRelayBeforeSync = true
    else if (arg === '--managed-target-count') options.managedTargetCount = Number(argv[++i])
    else if (arg === '--tracked-target-count') options.trackedTargetCount = Number(argv[++i])
    else if (arg === '--tracked-bidirectional') options.trackedBidirectional = true
    else if (arg === '--mode') {
      options.mode = argv[++i]
      if (!['off', 'managed', 'tracked'].includes(options.mode)) throw new Error('--mode must be off, managed, or tracked')
    }
    else if (arg === '--target-branch') options.targetBranch = argv[++i]
    else if (arg === '--hetzner-worker') options.hetznerWorker = true
    else if (arg === '--hetzner-host') options.hetznerHost = argv[++i]
    else if (arg === '--hetzner-key') options.hetznerKey = argv[++i]
    else if (arg === '--hetzner-repo') options.hetznerRepo = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!Number.isInteger(options.trackedTargetCount) || options.trackedTargetCount < 1) {
    throw new Error('--tracked-target-count must be a positive integer')
  }
  if (!Number.isInteger(options.managedTargetCount) || options.managedTargetCount < 0) {
    throw new Error('--managed-target-count must be a non-negative integer')
  }
  if (options.providers.includes('dev-stub')) {
    throw new Error('remote Workspace Live Sync drills require a real file-editing provider; use live-remote-machine-runtime-drill.mjs for dev-stub relay/lease validation')
  }
  return options
}

function preflightWorkspaceLiveSyncSupport(options) {
  if (options.hetznerWorker && options.mode === 'managed') {
    return {
      status: 'unsupported',
      mode: 'remote-workspace-live-sync-live-drill',
      liveSyncMode: options.mode,
      hetznerWorker: true,
      reason: HETZNER_MANAGED_WORKSPACE_LIVE_SYNC_UNSUPPORTED_REASON,
    }
  }
  return null
}

function makePorts() {
  const base = 58000 + Math.floor(Math.random() * 1000)
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

async function assertHetznerBinaries(options) {
  await assertHetznerArrobaBinaries(options)
}

function localCodexAuthPath() {
  const codexHome = process.env.CODEX_HOME?.trim() || path.join(os.homedir(), '.codex')
  return path.join(codexHome, 'auth.json')
}

async function syncHetznerCodexAuth(options) {
  const authPath = localCodexAuthPath()
  await access(authPath)
  await execFileAsync('ssh', sshArgs(options, 'mkdir -p /root/.codex && chmod 700 /root/.codex'))
  await execFileAsync('scp', [
    '-i',
    options.hetznerKey,
    '-o',
    'BatchMode=yes',
    '-o',
    'StrictHostKeyChecking=accept-new',
    authPath,
    `${options.hetznerHost}:/root/.codex/auth.json.tmp`,
  ])
  await execFileAsync('ssh', sshArgs(options, 'mv /root/.codex/auth.json.tmp /root/.codex/auth.json && chmod 600 /root/.codex/auth.json'))
}

async function assertHetznerRelayPortAvailable(options, port) {
  await assertHetznerTcpPortAvailable(options, port, 'Hetzner relay port')
}

async function stopOwnedHetznerRelay(options, port, runId) {
  await stopHetznerProcessByEnv(options, {
    ARROBA_WORKSPACE_LIVE_SYNC_DRILL_RUN_ID: runId,
    ARROBA_RELAY_PORT: String(port),
  })
}

function mirrorFixturesToHetznerCommand(options, rootDir) {
  const parent = path.dirname(rootDir)
  const base = path.basename(rootDir)
  const remoteCommand = [
    `rm -rf ${shellQuote(rootDir)}`,
    `mkdir -p ${shellQuote(parent)}`,
    `tar --no-same-owner -C ${shellQuote(parent)} -xf -`,
  ].join(' && ')
  return [
    `COPYFILE_DISABLE=1 tar -C ${shellQuote(parent)} -cf - ${shellQuote(base)}`,
    '|',
    'ssh',
    '-i',
    shellQuote(options.hetznerKey),
    '-o',
    'BatchMode=yes',
    '-o',
    'StrictHostKeyChecking=accept-new',
    shellQuote(options.hetznerHost),
    shellQuote(remoteCommand),
  ].join(' ')
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

async function waitForRemoteMachineKernel(client, machineRef, providers) {
  let lastError = null
  let last = []
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const payload = unwrapVariant(
        await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
        'RemoteMachineKernelsListed',
      )
      last = payload.kernels || []
      const kernel = last.find((candidate) => candidate.accepting_remote_leases
        && providers.every((provider) => (candidate.available_providers || []).includes(provider)))
      if (kernel) return kernel
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
    }
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not advertise providers ${providers.join(',')}; last=${JSON.stringify(last)} error=${lastError ?? 'unknown error'}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log('Usage: node apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs [--providers opencode,codex] [--model MODEL] [--provider-model PROVIDER=MODEL] [--full] [--mode off|managed|tracked] [--managed-target-count COUNT] [--target-branch BRANCH] [--tracked-target-count COUNT] [--tracked-bidirectional] [--restart-relay-before-sync] [--hetzner-worker]')
    console.log('Example: node apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs --provider codex --provider-model codex=gpt-5.2 --full')
    return
  }
  if (options.providers.length < 1) throw new Error('remote workspace live sync drill requires at least one provider')
  const unsupported = preflightWorkspaceLiveSyncSupport(options)
  if (unsupported) {
    console.error(`[workspace-live-sync-drill] unsupported: ${unsupported.reason}`)
    console.log(JSON.stringify(unsupported, null, 2))
    process.exitCode = 2
    return
  }

  const ports = makePorts()
  const runId = `${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), `arroba-remote-workspace-live-sync-${runId}`)
  const cliRuntimeDir = path.join(cliRoot, `.tmp-live-remote-workspace-live-sync-drill-${runId}`)
  await prepareDrillArtifacts(rootDir)
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  await buildKernelClient()
  const { LocalIpcClient, requests } = await loadCliModules(cliRuntimeDir)
  const { createSessionRequest, endSessionRequest, listRemoteMachinesRequest } = requests

  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const relayBinary = options.hetznerWorker
    ? null
    : await resolveBinary(
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
  const workerMachineId = `workspace-live-sync-machine-worker-${process.pid}`
  const workerMachineAlias = `workspace-live-sync-worker-${process.pid}`
  const homeDaemonId = `workspace-live-sync-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `workspace-live-sync-worker-${process.pid}-${Date.now()}`
  const homeHistoryDir = path.join(rootDir, `${homeDaemonId}-history`)
  const workerHistoryDir = path.join(rootDir, `${workerDaemonId}-history`)

  let relayChild = null
  let relayTunnel = null
  let homeChild = null
  let workerChild = null
  let localClient = null
  let succeeded = false
  let failure = null
  const childRootDir = options.hetznerWorker
    ? path.join(cliRoot, 'target', 'live-workspace-live-sync-drill', `hetzner-${runId}`)
    : null

  try {
    if (options.hetznerWorker) {
      await assertHetznerBinaries(options)
      if (options.providers.includes('codex')) {
        await syncHetznerCodexAuth(options)
      }
      await assertHetznerRelayPortAvailable(options, ports.relayPort)
    }
    if (options.hetznerWorker) {
      relayChild = spawn('ssh', sshArgs(options, remoteEnvCommand({
        ARROBA_REMOTE_REPO: options.hetznerRepo,
        ARROBA_RELAY_HOST: '127.0.0.1',
        ARROBA_RELAY_PORT: String(ports.relayPort),
        ARROBA_RELAY_TOKEN: relayToken,
        ARROBA_WORKSPACE_LIVE_SYNC_DRILL_RUN_ID: runId,
      }, './apps/relay/target/debug/arroba-relay')), { stdio: ['ignore', 'ignore', 'inherit'] })
      relayTunnel = spawn('ssh', [
        '-i',
        options.hetznerKey,
        '-o',
        'BatchMode=yes',
        '-o',
        'StrictHostKeyChecking=accept-new',
        '-N',
        '-L',
        `127.0.0.1:${ports.relayPort}:127.0.0.1:${ports.relayPort}`,
        options.hetznerHost,
      ], { stdio: ['ignore', 'ignore', 'inherit'] })
      await waitForTcpPort(ports.relayPort, '127.0.0.1', 30_000)
    } else {
      relayChild = spawn(relayBinary, [], { cwd: repoRoot, env: relayEnv, stdio: ['ignore', 'ignore', 'inherit'] })
      await waitForTcpPort(ports.relayPort)
    }
    homeChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: homeDaemonId,
        daemonAlias: 'home',
        machineId: `workspace-live-sync-machine-home-${process.pid}`,
        machineAlias: `workspace-live-sync-home-machine-${process.pid}`,
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
    if (options.hetznerWorker) {
      const remoteRoot = `/tmp/arroba-remote-workspace-live-sync-${runId}`
      workerChild = spawn('ssh', sshArgs(options, remoteEnvCommand({
        ARROBA_REMOTE_REPO: options.hetznerRepo,
        PATH: '/root/.bun/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
        HOME: '/root',
        XDG_CONFIG_HOME: '/root/.config',
        XDG_STATE_HOME: '/root/.local/state',
        XDG_DATA_HOME: '/root/.local/share',
        XDG_CACHE_HOME: '/root/.cache',
        CODEX_HOME: '/root/.codex',
        OPENCODE_CONFIG_DIR: '/root/.config/opencode',
        ARROBA_LOG_DIR: path.posix.join(remoteRoot, 'worker-logs'),
        ARROBA_KERNEL_PORT: String(ports.workerKernelPort),
        ARROBA_MCP_PORT: String(ports.workerMcpPort),
        ARROBA_OPENCODE_PORT: String(ports.workerOpenCodePort),
        ARROBA_CODEX_PORT: String(ports.workerCodexPort),
        ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
        ARROBA_RELAY_TOKEN: relayToken,
        ARROBA_DAEMON_ID: workerDaemonId,
        ARROBA_DAEMON_ALIAS: 'worker',
        ARROBA_MACHINE_ID: workerMachineId,
        ARROBA_MACHINE_ALIAS: workerMachineAlias,
        ARROBA_ACCEPT_REMOTE_LEASES: '1',
        ARROBA_DAEMON_SOCKET: path.posix.join(remoteRoot, 'worker.sock'),
        ARROBA_SESSION_HISTORY_DIR: path.posix.join(remoteRoot, 'worker-history'),
      }, `mkdir -p ${shellQuote(remoteRoot)} && ./apps/kernel/target/debug/arroba-kernel`)), {
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    } else {
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
        }),
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }

    await waitForLocalDaemon(LocalIpcClient, homeKernelUrl, createSessionRequest, endSessionRequest, repoRoot)
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'worker')

    localClient = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForRemoteMachine(localClient, listRemoteMachinesRequest, workerMachineId)
    let workerKernel = await waitForRemoteMachineKernel(localClient, workerMachineId, Array.from(new Set(options.providers)))
    if (options.restartRelayBeforeSync) {
      await terminateChild(relayChild)
      if (options.hetznerWorker) {
        await terminateChild(relayTunnel)
        await stopOwnedHetznerRelay(options, ports.relayPort, runId)
        await assertHetznerRelayPortAvailable(options, ports.relayPort)
        relayChild = spawn('ssh', sshArgs(options, remoteEnvCommand({
          ARROBA_REMOTE_REPO: options.hetznerRepo,
          ARROBA_RELAY_HOST: '127.0.0.1',
          ARROBA_RELAY_PORT: String(ports.relayPort),
          ARROBA_RELAY_TOKEN: relayToken,
          ARROBA_WORKSPACE_LIVE_SYNC_DRILL_RUN_ID: runId,
        }, './apps/relay/target/debug/arroba-relay')), { stdio: ['ignore', 'ignore', 'inherit'] })
        relayTunnel = spawn('ssh', [
          '-i',
          options.hetznerKey,
          '-o',
          'BatchMode=yes',
          '-o',
          'StrictHostKeyChecking=accept-new',
          '-N',
          '-L',
          `127.0.0.1:${ports.relayPort}:127.0.0.1:${ports.relayPort}`,
          options.hetznerHost,
        ], { stdio: ['ignore', 'ignore', 'inherit'] })
      } else {
        relayChild = spawn(relayBinary, [], { cwd: repoRoot, env: relayEnv, stdio: ['ignore', 'ignore', 'inherit'] })
      }
      await waitForTcpPort(ports.relayPort, '127.0.0.1', options.hetznerWorker ? 30_000 : 15_000)
      await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'home')
      await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'worker')
      await waitForRemoteMachine(localClient, listRemoteMachinesRequest, workerMachineId)
      workerKernel = await waitForRemoteMachineKernel(localClient, workerMachineId, Array.from(new Set(options.providers)))
    }

    const stdout = await runNodeDrillChild([
      path.join('apps', 'cli', 'scripts', 'live-workspace-live-sync-drill.mjs'),
      '--kernel', homeKernelUrl,
      '--no-spawn-daemon',
      '--machine-ref', workerMachineId,
      '--history-dir', homeHistoryDir,
      ...(options.hetznerWorker ? [] : ['--provider-history-dir', workerHistoryDir]),
      '--providers', options.providers.join(','),
      '--model', options.model,
      ...Object.entries(options.providerModels).flatMap(([provider, model]) => ['--provider-model', `${provider}=${model}`]),
      '--timeout-ms', String(options.timeoutMs),
      '--poll-ms', String(options.pollMs),
      '--mode', options.mode,
      '--managed-target-count', String(options.managedTargetCount),
      '--target-branch', options.targetBranch,
      '--tracked-target-count', String(options.trackedTargetCount),
      ...(childRootDir ? ['--root-dir', childRootDir] : []),
      ...(options.hetznerWorker && childRootDir ? ['--after-fixture-command', mirrorFixturesToHetznerCommand(options, childRootDir)] : []),
      ...(options.hetznerWorker && options.mode === 'tracked' ? ['--remote-source-side-effects'] : []),
      ...(options.trackedBidirectional ? ['--tracked-bidirectional'] : []),
      ...(options.full ? [] : ['--positive-only']),
      ...(options.keepArtifactsOnFailure ? ['--keep-artifacts-on-failure'] : []),
    ], repoRoot, { label: 'remote workspace live sync drill' })

    const trimmed = stdout.trim()
    const lastJsonIndex = trimmed.lastIndexOf('\n{')
    const jsonText = lastJsonIndex >= 0 ? trimmed.slice(lastJsonIndex + 1) : trimmed
    const result = JSON.parse(jsonText)
    if (options.hetznerWorker && options.mode === 'tracked' && childRootDir) {
      await assertHetznerTrackedSourceSideEffects(options, childRootDir, options.providers)
    }
    console.log(JSON.stringify({
      status: 'ok',
      mode: 'remote-workspace-live-sync-live-drill',
      relayUrl,
      homeKernelUrl,
      workerMachineId,
      workerMachineAlias,
      workerKernel: {
        kernelId: workerKernel.kernel_id,
        machineId: workerKernel.machine_id,
        providers: workerKernel.available_providers,
      },
      providers: options.providers,
      model: options.model,
      providerModels: options.providerModels,
      full: options.full,
      restartRelayBeforeSync: options.restartRelayBeforeSync,
      hetznerWorker: options.hetznerWorker,
      liveSyncMode: options.mode,
      managedTargetCount: options.managedTargetCount,
      targetBranch: options.targetBranch,
      trackedTargetCount: options.trackedTargetCount,
      trackedBidirectional: options.trackedBidirectional,
      workspaceLiveSync: result,
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
    await terminateChild(relayTunnel)
    if (options.hetznerWorker) {
      await stopOwnedHetznerRelay(options, ports.relayPort, runId)
    }
    if (options.hetznerWorker && childRootDir && (succeeded || !options.keepArtifactsOnFailure)) {
      await runHetznerCommand(options, `rm -rf ${shellQuote(childRootDir)}`).catch(() => {})
    }
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'remote-workspace-live-sync',
        mode: options.mode,
        providers: options.providers.join(','),
        managedTargetCount: options.managedTargetCount,
        trackedTargetCount: options.trackedTargetCount,
        trackedBidirectional: options.trackedBidirectional,
        restartRelayBeforeSync: options.restartRelayBeforeSync,
        hetznerWorker: options.hetznerWorker,
        childRootDir: childRootDir ?? '',
        cliRuntimeDir,
      },
      log: (name, details) => console.log(`[remote-workspace-live-sync-drill] ${name}`, JSON.stringify(details)),
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote workspace live sync drill transient CLI modules kept at ${cliRuntimeDir}`)
      if (childRootDir) console.error(`remote workspace live sync child drill artifacts kept at ${childRootDir}`)
    }
  }
}

async function assertHetznerTrackedSourceSideEffects(options, rootDir, providers) {
  const checks = []
  for (const provider of providers) {
    const ignoredPath = path.posix.join(rootDir, 'workspace', 'ignored', `${provider}-ignored.txt`)
    const siblingPath = path.posix.join(rootDir, 'sibling-repo', `${provider}-tracked-sibling.txt`)
    checks.push(
      `test "$(cat ${shellQuote(ignoredPath)})" = ${shellQuote(`${provider}-ignored`)}`,
      `test "$(cat ${shellQuote(siblingPath)})" = ${shellQuote(`${provider}-tracked-sibling`)}`,
    )
  }
  await runHetznerCommand(options, checks.join(' && '))
}

await main()

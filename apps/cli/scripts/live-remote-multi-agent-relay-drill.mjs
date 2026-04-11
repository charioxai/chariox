import { spawn } from 'node:child_process'
import { access, mkdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

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
let attachToSessionRequest
let getSessionStateRequest
let listRemoteMachinesRequest
let listAgentsRequest
let submitPromptRequest
let pumpTerminalOutputRequest
let endSessionRequest

const DEFAULT_PROVIDER = 'opencode'
const DEFAULT_MODEL = 'kimi-k2.5'
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function makePorts() {
  const base = 52000 + Math.floor(Math.random() * 1000)
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
  }
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ['ignore', 'ignore', 'inherit'] })
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

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
    const probe = new LocalIpcClient(kernelUrl)
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
    const client = new LocalIpcClient(relayUrl, { relayAuthToken: relayToken, targetDaemonAlias })
    try {
      await Promise.race([
        client.send(getSessionStateRequest('missing-session')),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (
        !message.includes('session_not_found')
        && !message.includes('SessionNotFound')
        && !message.includes('was not found')
      ) {
        lastError = message
        await client.close().catch(() => {})
        await sleep(250)
        continue
      }
    }
    await client.close().catch(() => {})
    return
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForRemoteMachine(client, machineRef) {
  let lastError = null
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
      if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return machines
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
    }
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become visible: ${lastError ?? 'unknown error'}`)
}

async function waitForEvent(bucket, predicate, timeoutMs, description) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const match = bucket.find(predicate)
    if (match) return match
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${description}`)
}

async function waitForCompletion(client, sessionId, attachmentId, bucket, baselineCount, timeoutMs, pollMs) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = bucket.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > baselineCount) return completions.at(-1)
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    await sleep(pollMs)
  }
  throw new Error('timed out waiting for assistant completion')
}

function remoteSpawnAgentRequest(sessionId, provider, alias, model, machineRef) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort: 'medium',
      worktree_id: repoRoot,
      machine_ref: machineRef,
    },
  }
}

function localSpawnAgentRequest(sessionId, provider, alias, model) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort: 'medium',
      worktree_id: repoRoot,
      machine_ref: null,
    },
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log('Usage: node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs [--provider opencode|codex] [--model MODEL]')
    return
  }

  const ports = makePorts()
  const rootDir = path.join(os.tmpdir(), `arroba-remote-multi-agent-${process.pid}-${Date.now()}`)
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-multi-agent-relay-drill')
  await mkdir(rootDir, { recursive: true })
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  ;({ LocalIpcClient, requests: {
    createSessionRequest,
    attachToSessionRequest,
    getSessionStateRequest,
    listRemoteMachinesRequest,
    listAgentsRequest,
    submitPromptRequest,
    pumpTerminalOutputRequest,
    endSessionRequest,
  } } = await loadCliModules(cliRuntimeDir))

  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: '127.0.0.1',
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_TOKEN: relayToken,
  }

  const homeDaemonId = `multi-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `multi-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `machine-worker-${process.pid}`
  const workerMachineAlias = `builder-west-${process.pid}`

  const relayBinary = await resolveBinary(
    path.join(repoRoot, 'apps/relay/target/debug/arroba-relay'),
    path.join(repoRoot, 'apps/relay/Cargo.toml'),
    'arroba-relay',
  )
  const daemonBinary = await resolveBinary(
    path.join(repoRoot, 'apps/daemon/target/debug/arroba-daemon'),
    path.join(repoRoot, 'apps/daemon/Cargo.toml'),
    'arroba-daemon',
  )

  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let localClient = null
  let remoteClient = null
  let sessionId = null

  try {
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    homeChild = spawnProcess(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: homeDaemonId,
        daemonAlias: 'home',
        machineId: `machine-home-${process.pid}`,
        machineAlias: `home-machine-${process.pid}`,
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
      }),
    })

    await waitForLocalDaemon(homeKernelUrl)
    await waitForRelayTarget(relayUrl, relayToken, 'home')
    await waitForRelayTarget(relayUrl, relayToken, 'worker')

    localClient = new LocalIpcClient(homeKernelUrl)
    remoteClient = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias: 'home',
    })

    const machines = await waitForRemoteMachine(remoteClient, workerMachineId)

    const localEvents = []
    const remoteEvents = []

    const session = unwrap(await localClient.send(createSessionRequest(repoRoot, repoRoot)), 'SessionCreated').session
    sessionId = session.id
    const localAttachment = unwrap(
      await localClient.send(attachToSessionRequest(session.id, `local-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    localClient.onKernelEvent((event) => localEvents.push({ ...event, source: 'local', observed_at_ms: Date.now() }))
    await localClient.subscribeToKernelEvents(session.id, localAttachment.id)

    const remoteAttachment = unwrap(
      await remoteClient.send(attachToSessionRequest(session.id, `remote-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    remoteClient.onKernelEvent((event) => remoteEvents.push({ ...event, source: 'remote', observed_at_ms: Date.now() }))
    await remoteClient.subscribeToKernelEvents(session.id, remoteAttachment.id)

    await waitForEvent(localEvents, (event) => event.event === 'session_snapshot', 30_000, 'local snapshot')
    await waitForEvent(remoteEvents, (event) => event.event === 'session_snapshot', 30_000, 'remote snapshot')

    const localExtraAgent = unwrapVariant(
      await remoteClient.send(localSpawnAgentRequest(session.id, options.provider, 'local-sidecar', options.model)),
      'AgentSpawned',
    ).agent
    const remoteAgentA = unwrapVariant(
      await localClient.send(remoteSpawnAgentRequest(session.id, options.provider, 'remote-a', options.model, workerMachineId)),
      'AgentSpawned',
    ).agent
    const remoteAgentB = unwrapVariant(
      await remoteClient.send(remoteSpawnAgentRequest(session.id, options.provider, 'remote-b', options.model, workerMachineId)),
      'AgentSpawned',
    ).agent

    const listed = unwrapVariant(await remoteClient.send(listAgentsRequest(session.id)), 'AgentsListed').agents || []
    if (listed.length < 4) {
      throw new Error(`expected at least 4 agents in session, got ${listed.length}`)
    }

    const baselineRemote1 = remoteEvents.filter((event) => event.event === 'assistant_message_completed').length
    await remoteClient.send(submitPromptRequest(
      session.id,
      remoteAttachment.id,
      localExtraAgent.id,
      'Reply with exactly LOCAL_SIDE_OK and nothing else.',
      [],
    ))
    await waitForCompletion(remoteClient, session.id, remoteAttachment.id, remoteEvents, baselineRemote1, options.timeoutMs, options.pollMs)
    const localObservedFirst = localEvents.filter((event) => event.event === 'assistant_message_completed').length

    const baselineLocal2 = localEvents.filter((event) => event.event === 'assistant_message_completed').length
    await localClient.send(submitPromptRequest(
      session.id,
      localAttachment.id,
      remoteAgentA.id,
      'Reply with exactly REMOTE_AGENT_A_OK and nothing else.',
      [],
    ))
    await waitForCompletion(localClient, session.id, localAttachment.id, localEvents, baselineLocal2, options.timeoutMs, options.pollMs)
    const remoteObservedSecond = remoteEvents.filter((event) => event.event === 'assistant_message_completed').length

    await terminateChild(relayChild)
    await waitForEvent(remoteEvents, (event) => event.event === 'transport_closed', 30_000, 'transport_closed')
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    await waitForRelayTarget(relayUrl, relayToken, 'home')
    await waitForRelayTarget(relayUrl, relayToken, 'worker')
    const resumed = await waitForEvent(remoteEvents, (event) => event.event === 'transport_resumed', 45_000, 'transport_resumed')

    const baselineRemote3 = remoteEvents.filter((event) => event.event === 'assistant_message_completed').length
    await remoteClient.send(submitPromptRequest(
      session.id,
      remoteAttachment.id,
      remoteAgentB.id,
      'Reply with exactly REMOTE_AGENT_B_OK and nothing else.',
      [],
    ))
    await waitForCompletion(remoteClient, session.id, remoteAttachment.id, remoteEvents, baselineRemote3, options.timeoutMs, options.pollMs)
    const localObservedThird = localEvents.filter((event) => event.event === 'assistant_message_completed').length

    const finalState = unwrapVariant(await remoteClient.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')

    console.log(JSON.stringify({
      status: 'ok',
      relayUrl,
      homeKernelUrl,
      sessionId: session.id,
      provider: options.provider,
      model: options.model,
      machinesVisible: machines.map((machine) => ({
        machineId: machine.machine_id,
        machineAlias: machine.machine_alias,
        providers: machine.available_providers,
      })),
      agents: {
        localExtraAgentId: localExtraAgent.id,
        remoteAgentAId: remoteAgentA.id,
        remoteAgentBId: remoteAgentB.id,
        remoteBindings: [remoteAgentA, remoteAgentB].map((agent) => agent.remote_execution ?? null),
      },
      events: {
        local: {
          total: localEvents.length,
          completed: localEvents.filter((event) => event.event === 'assistant_message_completed').length,
          sessionSnapshots: localEvents.filter((event) => event.event === 'session_snapshot').length,
        },
        remote: {
          total: remoteEvents.length,
          completed: remoteEvents.filter((event) => event.event === 'assistant_message_completed').length,
          sessionSnapshots: remoteEvents.filter((event) => event.event === 'session_snapshot').length,
          transportClosed: remoteEvents.filter((event) => event.event === 'transport_closed').length,
          transportResumed: remoteEvents.filter((event) => event.event === 'transport_resumed').length,
        },
      },
      relayReconnect: {
        resumedFromEventId: resumed.resumed_from_event_id ?? null,
      },
      crossObservation: {
        localSawRemoteClientPromptCompletion: localObservedFirst,
        remoteSawLocalClientPromptCompletion: remoteObservedSecond,
        localSawPostReconnectRemotePromptCompletion: localObservedThird,
      },
      finalFocusedAgentId: finalState.session?.focused_agent_id ?? null,
      listedAgentCount: listed.length,
    }, null, 2))
  } finally {
    if (remoteClient) {
      if (sessionId) await remoteClient.send(endSessionRequest(sessionId)).catch(() => {})
      await remoteClient.close().catch(() => {})
    }
    if (localClient) {
      await localClient.close().catch(() => {})
    }
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()

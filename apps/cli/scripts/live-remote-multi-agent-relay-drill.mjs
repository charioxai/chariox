import { spawn } from 'node:child_process'
import { access, mkdir, rm } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

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

const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 1_000

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
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
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(mcpPort),
    CHARIOX_OPENCODE_PORT: String(opencodePort),
    CHARIOX_CODEX_PORT: String(codexPort),
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_ALIAS: daemonAlias,
    CHARIOX_MACHINE_ID: machineId,
    CHARIOX_MACHINE_ALIAS: machineAlias,
    CHARIOX_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, socketName),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonId}-history`),
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonId}-xdg-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonId}-xdg-state`),
    XDG_CACHE_HOME: path.join(rootDir, `${daemonId}-xdg-cache`),
  }
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ['ignore', 'ignore', 'inherit'] })
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function eventCounts(events) {
  return events.reduce((counts, event) => {
    const key = event.event ?? 'unknown'
    counts[key] = (counts[key] ?? 0) + 1
    return counts
  }, {})
}

function describeRemoteKernelsForError(kernels) {
  return kernels.map((kernel) => ({
    kernel_id: kernel.kernel_id,
    machine_id: kernel.machine_id,
    machine_alias: kernel.machine_alias,
    kernel_alias: kernel.kernel_alias,
    available_providers: kernel.available_providers ?? [],
    accepting_remote_leases: kernel.accepting_remote_leases ?? false,
    leased_agent_count: kernel.leased_agent_count ?? 0,
    local_session_count: kernel.local_session_count ?? 0,
  }))
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  if (provider === 'codex' && !options.model.includes('/')) return opencodeCodexModel(options.model)
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
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

async function waitForRemoteKernel(client, machineRef, provider, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = []
  while (Date.now() < deadline) {
    const response = unwrapVariant(
      await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
      'RemoteMachineKernelsListed',
    )
    last = response.kernels || []
    const kernel = last.find((candidate) => candidate.accepting_remote_leases && (candidate.available_providers || []).includes(provider))
    if (kernel) return kernel
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not advertise provider ${provider}; last=${JSON.stringify(describeRemoteKernelsForError(last))}`)
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

function remoteSpawnAgentRequest(sessionId, provider, alias, model, kernelRef) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort: 'low',
      worktree_id: repoRoot,
      kernel_ref: kernelRef,
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
      effort: 'low',
      worktree_id: repoRoot,
    },
  }
}

function requireRemotePlacement(agent, workerKernel) {
  if (!agent.remote_execution?.leased_agent_id) {
    throw new Error(`agent ${agent.id} was expected to be remote-backed\n${JSON.stringify(agent, null, 2)}`)
  }
  if (agent.remote_execution.worker_kernel_id !== workerKernel.kernel_id) {
    throw new Error(`agent ${agent.id} ran on ${agent.remote_execution.worker_kernel_id}, expected ${workerKernel.kernel_id}`)
  }
  if (agent.remote_execution.worker_machine_id !== workerKernel.machine_id) {
    throw new Error(`agent ${agent.id} ran on machine ${agent.remote_execution.worker_machine_id}, expected ${workerKernel.machine_id}`)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log('Usage: node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs [--providers opencode,codex] [--model MODEL] [--provider-model PROVIDER=MODEL]')
    return
  }
  if (options.providers.length < 2) {
    throw new Error('remote multi-agent relay drill requires at least two providers')
  }

  const ports = makePorts()
  const rootDir = path.join(repoRoot, '.artifacts', 'live-remote-multi-agent-relay-drill', nowStamp())
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-multi-agent-relay-drill')
  await prepareDrillArtifacts(rootDir)
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const relayEnv = {
    ...process.env,
    CHARIOX_RELAY_HOST: '127.0.0.1',
    CHARIOX_RELAY_PORT: String(ports.relayPort),
    CHARIOX_RELAY_TOKEN: relayToken,
  }

  const homeDaemonId = `multi-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `multi-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `machine-worker-${process.pid}`
  const workerMachineAlias = `builder-west-${process.pid}`

  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let localClient = null
  let remoteClient = null
  let sessionId = null
  let passed = false
  let failure = null
  let machines = []
  let listed = []
  let finalState = null
  let resumed = null
  const localEvents = []
  const remoteEvents = []
  const localSidecars = []
  const remoteAgents = []
  const beforeReconnectResults = []
  const afterReconnectResults = []

  try {
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
    const relayBinary = await resolveBinary(
      path.join(repoRoot, 'apps/relay/target/debug/chariox-relay'),
      path.join(repoRoot, 'apps/relay/Cargo.toml'),
      'chariox-relay',
    )
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'chariox-kernel',
    )
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

    machines = await waitForRemoteMachine(remoteClient, workerMachineId)

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

    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const localSidecar = unwrapVariant(
        await remoteClient.send(localSpawnAgentRequest(session.id, provider, `local-${index + 1}-${provider}`, modelForProvider(provider, options))),
        'AgentSpawned',
      ).agent
      localSidecars.push(localSidecar)

      const workerKernel = await waitForRemoteKernel(remoteClient, workerMachineId, provider, options.timeoutMs, options.pollMs)
      const spawnClient = index % 2 === 0 ? localClient : remoteClient
      const remoteAgent = unwrapVariant(
        await spawnClient.send(remoteSpawnAgentRequest(session.id, provider, `remote-${index + 1}-${provider}`, modelForProvider(provider, options), workerKernel.kernel_id)),
        'AgentSpawned',
      ).agent
      requireRemotePlacement(remoteAgent, workerKernel)
      remoteAgents.push(remoteAgent)
    }

    listed = unwrapVariant(await remoteClient.send(listAgentsRequest(session.id)), 'AgentsListed').agents || []
    if (listed.length < 1 + localSidecars.length + remoteAgents.length) {
      throw new Error(`expected at least ${1 + localSidecars.length + remoteAgents.length} agents in session, got ${listed.length}`)
    }

    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const localBaseline = localEvents.filter((event) => event.event === 'assistant_message_completed').length
      const remoteBaseline = remoteEvents.filter((event) => event.event === 'assistant_message_completed').length
      const targetAgent = index % 2 === 0 ? localSidecars[index] : remoteAgents[index]
      const submitClient = index % 2 === 0 ? remoteClient : localClient
      const attachment = index % 2 === 0 ? remoteAttachment : localAttachment
      await submitClient.send(submitPromptRequest(
        session.id,
        attachment.id,
        targetAgent.id,
        `Reply with exactly ${provider.toUpperCase()}_BEFORE_RELAY_OK and nothing else.`,
        [],
      ))
      await waitForCompletion(submitClient, session.id, attachment.id, index % 2 === 0 ? remoteEvents : localEvents, index % 2 === 0 ? remoteBaseline : localBaseline, options.timeoutMs, options.pollMs)
      beforeReconnectResults.push({
        provider,
        targetAgentId: targetAgent.id,
        localCompleted: localEvents.filter((event) => event.event === 'assistant_message_completed').length,
        remoteCompleted: remoteEvents.filter((event) => event.event === 'assistant_message_completed').length,
      })
    }

    await terminateChild(relayChild)
    await waitForEvent(remoteEvents, (event) => event.event === 'transport_closed', 30_000, 'transport_closed')
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    await waitForRelayTarget(relayUrl, relayToken, 'home')
    await waitForRelayTarget(relayUrl, relayToken, 'worker')
    resumed = await waitForEvent(remoteEvents, (event) => event.event === 'transport_resumed', 45_000, 'transport_resumed')

    for (let index = 0; index < remoteAgents.length; index += 1) {
      const provider = options.providers[index]
      const baselineRemote = remoteEvents.filter((event) => event.event === 'assistant_message_completed').length
      await remoteClient.send(submitPromptRequest(
        session.id,
        remoteAttachment.id,
        remoteAgents[index].id,
        `Reply with exactly ${provider.toUpperCase()}_AFTER_RELAY_OK and nothing else.`,
        [],
      ))
      await waitForCompletion(remoteClient, session.id, remoteAttachment.id, remoteEvents, baselineRemote, options.timeoutMs, options.pollMs)
      afterReconnectResults.push({
        provider,
        targetAgentId: remoteAgents[index].id,
        localCompleted: localEvents.filter((event) => event.event === 'assistant_message_completed').length,
        remoteCompleted: remoteEvents.filter((event) => event.event === 'assistant_message_completed').length,
      })
    }

    finalState = unwrapVariant(await remoteClient.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')

    console.log(JSON.stringify({
      status: 'ok',
      relayUrl,
      homeKernelUrl,
      sessionId: session.id,
      providers: options.providers,
      model: options.model,
      providerModels: options.providerModels,
      machinesVisible: machines.map((machine) => ({
        machineId: machine.machine_id,
        machineAlias: machine.machine_alias,
        providers: machine.available_providers,
      })),
      agents: {
        localSidecars: localSidecars.map((agent) => ({
          id: agent.id,
          provider: agent.provider,
        })),
        remoteAgents: remoteAgents.map((agent) => ({
          id: agent.id,
          provider: agent.provider,
          remoteExecution: agent.remote_execution ?? null,
        })),
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
        beforeReconnect: beforeReconnectResults,
        afterReconnect: afterReconnectResults,
      },
      finalFocusedAgentId: finalState.session?.focused_agent_id ?? null,
      listedAgentCount: listed.length,
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
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
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'live-remote-multi-agent-relay',
        providers: options.providers,
        model: options.model,
        providerModels: options.providerModels,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        ports,
        relayUrl,
        homeKernelUrl,
        workerKernelUrl: `ws://127.0.0.1:${ports.workerKernelPort}`,
        homeDaemonId,
        workerDaemonId,
        workerMachineId,
        workerMachineAlias,
        sessionId,
        machineCount: machines.length,
        listedAgentCount: listed.length,
        localSidecars: localSidecars.map((agent) => ({ id: agent.id, provider: agent.provider })),
        remoteAgents: remoteAgents.map((agent) => ({
          id: agent.id,
          provider: agent.provider,
          remoteExecution: agent.remote_execution ?? null,
        })),
        events: {
          local: eventCounts(localEvents),
          remote: eventCounts(remoteEvents),
        },
        recentLocalEvents: localEvents.slice(-20),
        recentRemoteEvents: remoteEvents.slice(-20),
        relayReconnect: resumed ? { resumedFromEventId: resumed.resumed_from_event_id ?? null } : null,
        crossObservation: {
          beforeReconnect: beforeReconnectResults,
          afterReconnect: afterReconnectResults,
        },
        finalFocusedAgentId: finalState?.session?.focused_agent_id ?? null,
      },
    })
    await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()

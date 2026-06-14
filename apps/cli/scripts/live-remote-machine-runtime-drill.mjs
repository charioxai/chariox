import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { access, mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
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
  const ipcUrl = pathToFileURL(path.join(runtimeDir, 'ipc.js')).href
  const requestsUrl = pathToFileURL(path.join(runtimeDir, 'ipc-requests.js')).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

let LocalIpcClient
let createSessionRequest
let attachToSessionRequest
let getSessionStateRequest
let listSessionsRequest
let endSessionRequest
let submitPromptRequest

const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDER = 'dev-stub'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 1_000
const RELAY_ISSUER = 'arroba-remote-machine-runtime-drill'
const RELAY_SECRET = 'arroba-remote-machine-runtime-drill-secret'
const RELAY_REALM = 'remote-machine-runtime-drill'

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signRelayToken(claims) {
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac('sha256', RELAY_SECRET).update(payload).digest('base64url')
  return `arroba-scoped-v1.${payload}.${signature}`
}

function relayClaims({ subject, subjectKind, actions, userId = null, targets = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: 'remote-machine-runtime-drill-account',
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null,
    client_id: subjectKind === 'client' ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: 'drill',
  }
}

function parseArgs(argv) {
  const options = {
    model: DEFAULT_MODEL,
    providerModels: {},
    provider: DEFAULT_PROVIDER,
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--workspace') options.workspace = argv[++i]
    else if (arg === '--worktree') options.worktree = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
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

function makePorts() {
  const base = 47000 + Math.floor(Math.random() * 1000)
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

function makeDaemonEnv({ ports, rootDir, relayToken, daemonId, daemonAlias, machineId, machineAlias, acceptRemoteLeases, socketName, kernelPort, mcpPort, opencodePort, codexPort }) {
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((v) => v != null) ?? resp

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ['ignore', 'ignore', 'inherit'] })
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

async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, worktree)), 'SessionCreated').session
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

async function waitForRemoteMachine(localClient, machineRef) {
  let lastError = null
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await localClient.send({ ListRemoteMachineKernels: { machine_ref: machineRef } })
      const payload = unwrapVariant(response, 'RemoteMachineKernelsListed')
      if ((payload.kernels || []).length > 0) return payload.kernels
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
    }
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, { relayAuthToken: relayToken, targetDaemonAlias })
    try {
      await Promise.race([
        client.send(listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError ?? 'unknown error'}`)
}

async function waitForCompletion(eventLog, timeoutMs, baselineCount = 0) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = eventLog.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > baselineCount) return completions[completions.length - 1]
    await sleep(100)
  }
  throw new Error('timed out waiting for assistant completion')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log([
      'Usage: node apps/cli/scripts/live-remote-machine-runtime-drill.mjs [options]',
      '',
      'Options:',
      `  --provider ${DEFAULT_PROVIDER}`,
      `  --model ${DEFAULT_MODEL}`,
      '  --provider-model PROVIDER=MODEL (for example opencode=opencode/gpt-5.2)',
      `  --workspace ${DEFAULT_WORKSPACE}`,
      `  --worktree ${DEFAULT_WORKTREE}`,
      `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
      `  --poll-ms ${DEFAULT_POLL_MS}`,
    ].join('\n'))
    return
  }

  const ports = makePorts()
  const rootDir = path.join(repoRoot, '.artifacts', 'live-remote-machine-runtime-drill', nowStamp())
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-machine-runtime-drill')
  await prepareDrillArtifacts(rootDir)
  await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(cliRuntimeDir, { recursive: true })

  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: '127.0.0.1',
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
  }
  const homeDaemonId = `remote-machine-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `remote-machine-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `machine-worker-${process.pid}`
  const workerMachineAlias = `builder-west-${process.pid}`
  const clientRelayToken = signRelayToken(relayClaims({
    subject: `remote-machine-client-${process.pid}-${Date.now()}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId: 'local',
  }))
  const homeRelayToken = signRelayToken(relayClaims({
    subject: homeDaemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event', 'client_metadata_read'],
    userId: 'local',
  }))
  const workerRelayToken = signRelayToken(relayClaims({
    subject: workerDaemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event', 'client_metadata_read'],
    userId: 'local',
  }))

  const homeEnv = makeDaemonEnv({
    ports,
    rootDir,
    relayToken: homeRelayToken,
    daemonId: homeDaemonId,
    daemonAlias: 'home',
    machineId: `machine-home-${process.pid}`,
    machineAlias: `home-machine-${process.pid}`,
    acceptRemoteLeases: false,
    socketName: 'home-daemon.sock',
    kernelPort: ports.homeKernelPort,
    mcpPort: ports.homeMcpPort,
    opencodePort: ports.homeOpenCodePort,
    codexPort: ports.homeCodexPort,
  })
  const workerEnv = makeDaemonEnv({
    ports,
    rootDir,
    relayToken: workerRelayToken,
    daemonId: workerDaemonId,
    daemonAlias: 'worker',
    machineId: workerMachineId,
    machineAlias: workerMachineAlias,
    acceptRemoteLeases: true,
    socketName: 'worker-daemon.sock',
    kernelPort: ports.workerKernelPort,
    mcpPort: ports.workerMcpPort,
    opencodePort: ports.workerOpenCodePort,
    codexPort: ports.workerCodexPort,
  })

  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  let relayChild = null
  let homeChild = null
  let workerChild = null
  let client = null
  let sessionId = null
  let eventLog = []
  let cliDisplay = null
  let passed = false
  let failure = null
  let remoteMachines = []
  let kernels = []
  let selectedKernel = null
  let remoteAgent = null
  let remoteExecution = null
  let finalState = null

  try {
    ;({ LocalIpcClient, requests: {
      createSessionRequest,
      attachToSessionRequest,
      getSessionStateRequest,
      listSessionsRequest,
      endSessionRequest,
      submitPromptRequest,
    } } = await loadCliModules(cliRuntimeDir))
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
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    homeChild = spawnProcess(daemonBinary, [], { cwd: repoRoot, env: homeEnv })
    workerChild = spawnProcess(daemonBinary, [], { cwd: repoRoot, env: workerEnv })

    await waitForLocalDaemon(homeKernelUrl, options.workspace, options.worktree)
    await waitForRelayTarget(relayUrl, clientRelayToken, 'home')
    await waitForRelayTarget(relayUrl, clientRelayToken, 'worker')
    client = new LocalIpcClient(homeKernelUrl)
    await client.send({ ConfigureRelay: { relay_url: relayUrl, relay_token: homeRelayToken } })
    await waitForRemoteMachine(client, workerMachineId)

    const created = unwrap(await client.send(createSessionRequest(options.workspace, options.worktree)), 'SessionCreated')
    sessionId = created.session.id
    const defaultAgentId = created.agent.id
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `remote-machine-drill-${Date.now()}`)), 'SessionAttached').attachment
    client.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })
    await client.subscribeToKernelEvents(sessionId, attachment.id)

    const machineList = unwrapVariant(await client.send({ ListRemoteMachines: {} }), 'RemoteMachinesListed')
    remoteMachines = machineList.machines || []
    if (!remoteMachines.some((machine) => machine.machine_id === workerMachineId)) {
      throw new Error(`machine ${workerMachineId} not visible through home kernel`)
    }

    const kernelList = unwrapVariant(
      await client.send({ ListRemoteMachineKernels: { machine_ref: workerMachineId } }),
      'RemoteMachineKernelsListed',
    )
    kernels = kernelList.kernels || []
    selectedKernel = kernels.find((kernel) => kernel.accepting_remote_leases && (kernel.available_providers || []).includes(options.provider))
    if (!selectedKernel) {
      throw new Error(`no worker kernel on ${workerMachineId} advertises provider ${options.provider}`)
    }

    const [{ createWaitingRoomState, waitingRoomRows }, { fallbackProviderCatalog }] = await Promise.all([
      import('../dist/waiting-room.js'),
      import('../dist/provider-catalog.js'),
    ])
    const catalog = fallbackProviderCatalog()
    const relayStatus = unwrapVariant(await client.send({ RelayStatus: null }), 'RelayStatus').status
    const displayState = createWaitingRoomState([], catalog, 'opencode', 'opencode/gpt-5.4', 'low')
    const displayRows = waitingRoomRows(displayState, [], catalog, {
      relay: relayStatus,
      machines: remoteMachines,
      kernels,
    })
    const displayedKernelRow = displayRows.find((row) => row.id === `remote-kernel:${selectedKernel.kernel_id}`)
    if (!displayedKernelRow) {
      throw new Error(`waiting room rows did not include remote kernel ${selectedKernel.kernel_id}\n${JSON.stringify(displayRows, null, 2)}`)
    }
    cliDisplay = {
      row: displayedKernelRow,
      rowCount: displayRows.length,
    }

    const providerModel = modelForProvider(options.provider, options)
    const spawned = unwrapVariant(await client.send({
      SpawnAgent: {
        session_id: sessionId,
        alias: 'remote-reviewer',
        provider: options.provider,
        model: providerModel,
        effort: 'low',
        worktree_id: null,
        kernel_ref: selectedKernel.kernel_id,
      },
    }), 'AgentSpawned')
    remoteAgent = spawned.agent

    const sourceAttachmentPath = path.join(rootDir, 'remote-attachment.txt')
    await writeFile(sourceAttachmentPath, 'relay remote attachment\n', 'utf8')

    await client.send(submitPromptRequest(
      sessionId,
      attachment.id,
      remoteAgent.id,
      'Read the attached file and reply with exactly REMOTE_MACHINE_OK.',
      [{
        url: pathToFileURL(sourceAttachmentPath).href,
        mime: 'text/plain',
        filename: 'remote-attachment.txt',
      }],
    ))

    const started = Date.now()
    while (Date.now() - started < options.timeoutMs / 3) {
      if ((eventLog.filter((event) => event.event === 'terminal_output')).length > 0) break
      await sleep(options.pollMs)
    }

    let completeResponse = null
    try {
      completeResponse = unwrapVariant(
        await client.send({ CompletePrompt: { session_id: sessionId } }),
        'PromptCompleted',
      )
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (!message.includes('has no active prompt')) throw error
    }
    const firstCompletionEvent = await waitForCompletion(eventLog, options.timeoutMs, 0)

    await client.send(submitPromptRequest(
      sessionId,
      attachment.id,
      remoteAgent.id,
      'Start another remote prompt that will be cancelled.',
      [],
    ))
    const cancelled = unwrapVariant(
      await client.send({ CancelActivePrompt: { session_id: sessionId, attachment_id: attachment.id } }),
      'PromptCancelled',
    )

    finalState = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const finalAgent = finalState.session?.agents?.find((agent) => agent.id === remoteAgent.id) ?? remoteAgent
    remoteExecution = finalAgent.remote_execution ?? null
    if (!remoteExecution?.leased_agent_id) {
      throw new Error(`spawned agent ${remoteAgent.id} did not have remote execution placement\n${JSON.stringify(finalAgent, null, 2)}`)
    }
    if (remoteExecution.worker_kernel_id !== selectedKernel.kernel_id) {
      throw new Error(`spawned agent ${remoteAgent.id} targeted ${selectedKernel.kernel_id} but ran on ${remoteExecution.worker_kernel_id}`)
    }
    if (remoteExecution.worker_machine_id !== workerMachineId) {
      throw new Error(`spawned agent ${remoteAgent.id} targeted machine ${workerMachineId} but ran on ${remoteExecution.worker_machine_id}`)
    }

    console.log(JSON.stringify({
      status: 'ok',
      relayUrl: `ws://127.0.0.1:${ports.relayPort}`,
      homeKernelUrl,
      sessionId,
      workerMachineId,
      remoteAgentId: remoteAgent.id,
      providerModel,
      remoteExecution,
      selectedKernel: {
        kernelId: selectedKernel.kernel_id,
        machineId: selectedKernel.machine_id,
        providers: selectedKernel.available_providers,
      },
      terminalCliDisplay: {
        rowId: cliDisplay?.row?.id ?? null,
        title: cliDisplay?.row?.title ?? null,
        value: cliDisplay?.row?.value ?? null,
        rowCount: cliDisplay?.rowCount ?? 0,
      },
      firstPrompt: {
        completePromptResponse: completeResponse?.completion?.completed?.id ?? null,
        completionEventMessageId: firstCompletionEvent.message_id ?? null,
        terminalOutputEvents: eventLog.filter((event) => event.event === 'terminal_output').length,
        pumpedOutputRecords: 0,
      },
      secondPrompt: {
        cancelledPromptId: cancelled.cancellation?.prompt?.id ?? cancelled.prompt?.id ?? null,
        cancelledStatus: cancelled.cancellation?.prompt?.status ?? cancelled.prompt?.status ?? null,
      },
      machinesVisible: remoteMachines.map((machine) => ({
        machineId: machine.machine_id,
        machineAlias: machine.machine_alias,
        providers: machine.available_providers,
      })),
      finalFocusedAgentId: finalState.session?.focused_agent_id ?? null,
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) {
      if (sessionId) await client.send(endSessionRequest(sessionId)).catch(() => {})
      await client.close().catch(() => {})
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
        drill: 'live-remote-machine-runtime',
        provider: options.provider,
        model: options.model,
        providerModels: options.providerModels,
        workspace: options.workspace,
        worktree: options.worktree,
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
        remoteAgentId: remoteAgent?.id ?? null,
        remoteExecution,
        selectedKernel: selectedKernel ? {
          kernelId: selectedKernel.kernel_id,
          machineId: selectedKernel.machine_id,
          providers: selectedKernel.available_providers,
        } : null,
        machineCount: remoteMachines.length,
        kernelCount: kernels.length,
        terminalCliDisplay: cliDisplay,
        finalFocusedAgentId: finalState?.session?.focused_agent_id ?? null,
        eventCounts: eventLog.reduce((counts, event) => {
          counts[event.event ?? 'unknown'] = (counts[event.event ?? 'unknown'] ?? 0) + 1
          return counts
        }, {}),
        recentEvents: eventLog.slice(-30),
      },
    })
    await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()

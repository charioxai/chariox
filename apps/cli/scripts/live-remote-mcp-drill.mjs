import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const realHomeDir = os.homedir()

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
    liveMcpUse: false,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--live-mcp-use') options.liveMcpUse = true
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
    '- optionally verifies provider-native remote Playwright MCP use',
    '',
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --effort low',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --live-mcp-use',
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
    CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, '.codex'),
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, '.config', 'opencode'),
    XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME ?? path.join(realHomeDir, '.config'),
    XDG_DATA_HOME: process.env.XDG_DATA_HOME ?? path.join(realHomeDir, '.local', 'share'),
    XDG_CACHE_HOME: process.env.XDG_CACHE_HOME ?? path.join(realHomeDir, '.cache'),
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

async function waitForRemoteKernel(client, machineRef, provider) {
  let last = []
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const response = unwrapVariant(
      await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
      'RemoteMachineKernelsListed',
    )
    last = response.kernels || []
    const kernel = last.find((candidate) => candidate.accepting_remote_leases && (candidate.available_providers || []).includes(provider))
    if (kernel) return kernel
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not advertise provider ${provider}; last=${JSON.stringify(last)}`)
}

function spawnRemoteAgentRequest(sessionId, provider, alias, model, worktreeId, effort, kernelRef) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort,
      worktree_id: worktreeId,
      kernel_ref: kernelRef,
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

function playwrightMcpConfig() {
  return {
    name: 'playwright',
    transport: {
      type: 'stdio',
      command: 'npx',
      args: ['-y', '@playwright/mcp@latest'],
    },
    enabled: true,
    required: false,
    startup_timeout_sec: 45,
    tool_timeout_sec: 45,
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

async function expectGrant(label, fn) {
  const granted = unwrapVariant(await fn(), 'AgentExtensionGranted')
  if (!granted.agent) {
    throw new Error(`${label} did not return an updated agent`)
  }
  return { ok: true, agent: { id: granted.agent.id, ref: granted.agent.agent_ref, extension_grants: granted.agent.extension_grants } }
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

async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, timeoutMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (completed.length >= expectedCompletionCount) return completed
    await sleep(1000)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} assistant completions`)
}

async function waitForFileText({ filePath, requiredText, timeoutMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const text = await readFile(filePath, 'utf8').catch(() => '')
    if (text.includes(requiredText)) return text
    await sleep(1000)
  }
  throw new Error(`timed out waiting for ${requiredText} in ${filePath}`)
}

async function waitForHistoryToolCall({ historyDir, timeoutMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const files = (await readdir(historyDir).catch(() => []))
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(historyDir, file))
    for (const file of files) {
      const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
      for (const line of lines) {
        if (!line.trim()) continue
        let entry
        try {
          entry = JSON.parse(line)
        } catch {
          continue
        }
        if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
        let update
        try {
          update = JSON.parse(entry.text)
        } catch {
          continue
        }
        const tool = String(update.tool ?? '').toLowerCase()
        if (update.status === 'completed' && !tool.includes('arroba') && (tool.includes('playwright') || tool.includes('browser'))) {
          return update
        }
      }
    }
    await sleep(1000)
  }
  throw new Error('timed out waiting for provider-native Playwright/browser MCP tool call in worker history')
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
    grantAgentExtensionRequest,
    submitPromptRequest,
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
    const workerKernel = await waitForRemoteKernel(client, workerMachineId, options.provider)

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    const attachment = unwrap(await client.send(attachToSessionRequest(session.id, `remote-mcp-drill-${Date.now()}`)), 'SessionAttached').attachment

    const matching = mcpConfig('remote-mcp-drill', 'node', { args: ['--version'] })
    const mismatch = mcpConfig('remote-mcp-drill', 'node', { args: ['--eval', 'console.log("mismatch")'] })
    const missingCommand = mcpConfig('remote-mcp-missing-command', 'arroba-definitely-missing-mcp-command')
    const missingEnv = mcpConfig('remote-mcp-missing-env', 'node', { env_vars: ['ARROBA_REMOTE_MCP_DRILL_REQUIRED_ENV'] })

    const scenarioResults = []
    const events = []
    if (options.liveMcpUse) {
      client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
      await client.subscribeToKernelEvents(session.id, attachment.id)
    }

    await writeMcp(homeHomeDir, matching)
    const missingWorkerAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-mcp-missing-worker-agent',
      options.model,
      workspace,
      options.effort,
      workerKernel.kernel_id,
    )), 'AgentSpawned').agent
    requireRemotePlacement(missingWorkerAgent, workerKernel)
    scenarioResults.push({
      scenario: 'home_proxy_mcp_grant_ignores_missing_worker_definition',
      ...(await expectGrant(
        'home proxy MCP with missing worker definition',
        () => client.send(grantAgentExtensionRequest(workspace, missingWorkerAgent.id, 'mcp', 'remote-mcp-drill')),
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
      workerKernel.kernel_id,
    )), 'AgentSpawned').agent
    requireRemotePlacement(mismatchAgent, workerKernel)
    scenarioResults.push({
      scenario: 'home_proxy_mcp_grant_ignores_worker_definition_mismatch',
      ...(await expectGrant(
        'home proxy MCP with worker definition mismatch',
        () => client.send(grantAgentExtensionRequest(workspace, mismatchAgent.id, 'mcp', 'remote-mcp-drill')),
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
      workerKernel.kernel_id,
    )), 'AgentSpawned').agent
    requireRemotePlacement(overrideAgent, workerKernel)
    const overrideGranted = unwrapVariant(await client.send(grantAgentExtensionRequest(
      workspace,
      overrideAgent.id,
      'mcp',
      'remote-mcp-drill',
    )), 'AgentExtensionGranted').agent
    scenarioResults.push({
      scenario: 'worker_project_local_override_passes',
      ok: true,
      agent: { id: overrideGranted.id, ref: overrideGranted.agent_ref, extension_grants: overrideGranted.extension_grants },
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
      workerKernel.kernel_id,
    )), 'AgentSpawned').agent
    requireRemotePlacement(missingCommandAgent, workerKernel)
    scenarioResults.push({
      scenario: 'home_proxy_mcp_grant_defers_missing_command_to_invocation',
      ...(await expectGrant(
        'home proxy MCP grant with missing command',
        () => client.send(grantAgentExtensionRequest(workspace, missingCommandAgent.id, 'mcp', 'remote-mcp-missing-command')),
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
      workerKernel.kernel_id,
    )), 'AgentSpawned').agent
    requireRemotePlacement(missingEnvAgent, workerKernel)
    scenarioResults.push({
      scenario: 'home_proxy_mcp_grant_defers_missing_env_to_invocation',
      ...(await expectGrant(
        'home proxy MCP grant with missing env',
        () => client.send(grantAgentExtensionRequest(workspace, missingEnvAgent.id, 'mcp', 'remote-mcp-missing-env')),
      )),
    })

    if (options.liveMcpUse) {
      await writeMcp(homeHomeDir, playwrightMcpConfig())
      await writeMcp(workspace, playwrightMcpConfig())
      const liveAgent = unwrapVariant(await client.send(spawnRemoteAgentRequest(
        session.id,
        options.provider,
        'remote-mcp-live-playwright-agent',
        options.model,
        workspace,
        options.effort,
        workerKernel.kernel_id,
      )), 'AgentSpawned').agent
      requireRemotePlacement(liveAgent, workerKernel)
      await client.send(grantAgentExtensionRequest(workspace, liveAgent.id, 'mcp', 'playwright'))
      const markerFile = path.join(workspace, 'outputs', 'remote-playwright-mcp.txt')
      await rm(markerFile, { force: true }).catch(() => {})
      const before = events.filter((event) => event.event === 'assistant_message_completed').length
      await client.send(submitPromptRequest(session.id, attachment.id, liveAgent.id, [
        'This is a remote live MCP grant drill.',
        'Use the provider-native Playwright MCP tool that is available to this remote agent, not Arroba list_extensions/request_extension.',
        'The tool is usually named `mcp__playwright__browser_navigate`, `mcp__playwright__browser_snapshot`, `browser_navigate`, or similar.',
        'Prefer a non-mutating browser snapshot/title/text tool first; navigating to https://example.com is optional.',
        'After any Playwright/browser MCP tool call completes successfully, use Arroba workspace live sync to write `outputs/remote-playwright-mcp.txt` with exactly `M7_REMOTE_PLAYWRIGHT_MCP_OK`.',
        'Then reply exactly M7_REMOTE_PLAYWRIGHT_MCP_DONE.',
        'If Playwright MCP is unavailable, reply exactly M7_REMOTE_PLAYWRIGHT_MCP_UNAVAILABLE and do not write the marker file.',
      ].join('\n'), []))
      await waitForCompletionCount({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        expectedCompletionCount: before + 1,
        timeoutMs: options.timeoutMs,
      })
      const providerTool = await waitForHistoryToolCall({
        historyDir: workerHistoryDir,
        timeoutMs: options.timeoutMs,
      })
      const markerText = (await waitForFileText({
        filePath: markerFile,
        requiredText: 'M7_REMOTE_PLAYWRIGHT_MCP_OK',
        timeoutMs: options.timeoutMs,
      })).trim()
      if (markerText !== 'M7_REMOTE_PLAYWRIGHT_MCP_OK') {
        throw new Error(`unexpected remote Playwright MCP marker content: ${JSON.stringify(markerText)}`)
      }
      scenarioResults.push({
        scenario: 'remote_provider_native_playwright_mcp_use',
        ok: true,
        agent: { id: liveAgent.id, ref: liveAgent.agent_ref },
        markerFile,
        providerTool,
      })
    }

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
      liveMcpUse: options.liveMcpUse,
      completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
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

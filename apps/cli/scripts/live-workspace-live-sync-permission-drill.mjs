#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { chmod, copyFile, mkdir, rm, stat, readFile, readdir, symlink } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    provider: 'codex',
    model: null,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    useRealHome: false,
    kernelUrl: null,
    noSpawnDaemon: false,
    machineRef: null,
    rootDir: null,
    historyDir: null,
    afterFixtureCommand: null,
    workspaceFileCheckCommand: null,
    outsideFileCheckCommand: null,
    mode: 'managed',
    effort: null,
    providerModels: {},
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--use-real-home') options.useRealHome = true
    else if (arg === '--kernel') options.kernelUrl = argv[++i]
    else if (arg === '--no-spawn-daemon') options.noSpawnDaemon = true
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--root-dir') options.rootDir = argv[++i]
    else if (arg === '--history-dir') options.historyDir = argv[++i]
    else if (arg === '--after-fixture-command') options.afterFixtureCommand = argv[++i]
    else if (arg === '--workspace-file-check-command') options.workspaceFileCheckCommand = argv[++i]
    else if (arg === '--outside-file-check-command') options.outsideFileCheckCommand = argv[++i]
    else if (arg === '--mode') options.mode = argv[++i]
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs [options]',
        '',
        'Options:',
        '  --provider <codex|opencode>',
        '  --model <provider model override>',
        '  --provider-model <provider=model>',
        '  --effort <provider effort override>',
        '  --kernel <ws://...> (reuse an already-running kernel)',
        '  --no-spawn-daemon',
        '  --machine-ref <remote machine id or alias>',
        '  --root-dir <path>',
        '  --history-dir <path>',
        '  --after-fixture-command <cmd>',
        '  --workspace-file-check-command <cmd>',
        '  --outside-file-check-command <cmd>',
        '  --mode <managed|tracked>',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
        '  --use-real-home',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (!['managed', 'tracked'].includes(options.mode)) throw new Error(`unsupported live sync permission mode: ${options.mode}`)
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[workspace-live-sync-permission-drill] ${name}`)
  else console.log(`[workspace-live-sync-permission-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function makePorts() {
  const kernelPort = 52000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function defaultModelForProvider(provider) {
  if (provider === 'opencode') return 'opencode/gpt-5.4'
  return 'gpt-5.4'
}

function cliModelForProvider(provider, model) {
  if (provider === 'opencode' && model.startsWith('opencode/')) return model.slice('opencode/'.length)
  return model
}

function defaultEffortForProvider(provider) {
  if (provider.startsWith('claude')) return 'low'
  return 'high'
}

function liveSyncWriteToolName(provider) {
  if (provider === 'opencode') return 'arroba_write_artifact'
  return 'mcp__arroba__write_artifact'
}

async function seedCodexAuth(home) {
  const sourceHome = process.env.CODEX_HOME?.trim() || path.join(os.homedir(), '.codex')
  const sourceAuth = path.join(sourceHome, 'auth.json')
  const targetDir = path.join(home, '.codex')
  const targetAuth = path.join(targetDir, 'auth.json')
  await stat(sourceAuth)
  await mkdir(targetDir, { recursive: true })
  await copyFile(sourceAuth, targetAuth)
  await chmod(targetAuth, 0o600).catch(() => {})
}

async function seedClaudeAuth(home) {
  const sourceHome = os.homedir()
  await stat(path.join(sourceHome, '.claude'))
    .then(() => symlink(path.join(sourceHome, '.claude'), path.join(home, '.claude'), 'dir'))
    .catch(() => {})
  await stat(path.join(sourceHome, '.claude.json'))
    .then(() => symlink(path.join(sourceHome, '.claude.json'), path.join(home, '.claude.json')))
    .catch(() => {})
}

async function run(command, args, options = {}) {
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

async function initGitRepo(dir, label) {
  const result = await run('git', ['init', '-q'], { cwd: dir })
  if (result.code !== 0) {
    throw new Error(`${label} git init failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
}

async function ensureCliBuilt() {
  const cliDist = path.join(repoRoot, 'apps/cli/dist/index.js')
  const kernelBinary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const cliBuild = await run('pnpm', ['--filter', '@arroba/cli', 'run', 'build'])
  if (cliBuild.code !== 0) throw new Error(`cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
  const kernelBuild = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (kernelBuild.code !== 0) throw new Error(`kernel build failed\n${kernelBuild.stdout}\n${kernelBuild.stderr}`)
  await stat(cliDist)
  await stat(kernelBinary)
  return { cliDist, kernelBinary }
}

async function waitForKernel(kernelUrl) {
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { listSessionsRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForSocket(socketPath) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const client = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        client.once('connect', resolve)
        client.once('error', reject)
      })
      client.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding('utf8')
  let nextId = 1
  let buffer = ''
  const pending = new Map()
  socket.on('data', (chunk) => {
    buffer += chunk
    while (buffer.includes('\n')) {
      const newline = buffer.indexOf('\n')
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? 'automation command failed'))
    }
  })
  socket.on('error', (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2000)])
  }
}

async function pumpTerminalOutput(client, sessionId, attachmentId) {
  if (!attachmentId) return
  await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
}

async function readProviderLaunchFailure(historyDir) {
  let names = []
  try {
    names = await readdir(historyDir)
  } catch {
    return null
  }
  for (const name of names.filter((entry) => entry.endsWith('.jsonl'))) {
    let content = ''
    try {
      content = await readFile(path.join(historyDir, name), 'utf8')
    } catch {
      continue
    }
    for (const line of content.split('\n')) {
      if (!line.trim()) continue
      try {
        const entry = JSON.parse(line)
        const text = String(entry.text ?? '')
        if (entry.kind === 'notice' && text.includes('failed before it became ready')) {
          return text
        }
        if (entry.kind === 'provider_error') {
          return text || 'provider reported an error'
        }
      } catch {}
    }
  }
  return null
}

async function waitForSessionInteraction(client, sessionId, attachmentId, agentId, containsText, timeoutMs, pollMs, failureProbe = null) {
  const { getSessionStateRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + timeoutMs
  let session = null
  while (Date.now() < deadline) {
    await pumpTerminalOutput(client, sessionId, attachmentId)
    const response = await client.send(getSessionStateRequest(sessionId))
    const payload = response?.SessionStateLoaded ?? response?.SessionState ?? response
    session = payload.session ?? payload
    const interaction = (session.active_interactions ?? []).find((entry) => entry.agent_id === agentId && String(entry.message ?? '').includes(containsText))
      ?? (session.active_interactions ?? []).find((entry) => String(entry.message ?? '').includes(containsText))
    if (interaction) return { session, interaction }
    const failure = failureProbe ? await failureProbe() : null
    if (failure) throw new Error(failure)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for session interaction containing ${containsText}${session ? `\n${JSON.stringify(session, null, 2)}` : ''}`)
}

async function allowOutsideRepoProviderPermissions(client, sessionId, attachmentId, agentId, outsideRepo, outsideExpectedContent, respondedInteractionIds, pollMs) {
  const { getSessionStateRequest, respondToInteractionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  await pumpTerminalOutput(client, sessionId, attachmentId)
  const response = await client.send(getSessionStateRequest(sessionId))
  const payload = response?.SessionStateLoaded ?? response?.SessionState ?? response
  const session = payload.session ?? payload
  const activeInteractions = session.active_interactions ?? []
  const matchingInteractions = activeInteractions.filter((entry) => {
    if (entry.agent_id !== agentId || entry.kind !== 'permission' || respondedInteractionIds.has(entry.id)) return false
    const message = String(entry.message ?? '')
    return message.includes(outsideRepo) || message.includes(outsideExpectedContent)
  })
  for (const interaction of matchingInteractions) {
    log('answering-outside-repo-provider-interaction', {
      interactionId: interaction.id,
      level: interaction.level,
      title: interaction.title,
    })
    respondedInteractionIds.add(interaction.id)
    await client.send(respondToInteractionRequest(sessionId, interaction.id, 'allow_once'))
    await sleep(pollMs)
  }
}

async function fileContentMatches(filePath, expectedContent, checkCommand = null) {
  if (checkCommand) {
    const result = await run('/bin/sh', ['-lc', checkCommand])
    return result.code === 0
  }
  try {
    const content = await readFile(filePath, 'utf8')
    return content.trim() === expectedContent
  } catch {
    return false
  }
}

async function waitForFileContent(client, sessionId, attachmentId, filePath, expectedContent, timeoutMs, pollMs, failureProbe = null, checkCommand = null) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await pumpTerminalOutput(client, sessionId, attachmentId)
    if (await fileContentMatches(filePath, expectedContent, checkCommand)) return expectedContent
    const failure = failureProbe ? await failureProbe() : null
    if (failure) throw new Error(failure)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for file ${filePath}`)
}

async function waitForOutsideRepoFileContent(client, sessionId, attachmentId, agentId, provider, outsideRepo, filePath, expectedContent, timeoutMs, pollMs, failureProbe = null, checkCommand = null) {
  const { sendTerminalInputRequest } = await import('../../../packages/kernel-client/dist/ipc-terminal-runtime-requests.js')
  const deadline = Date.now() + timeoutMs
  const respondedInteractionIds = new Set()
  let lastClaudeNativeApprovalMs = 0
  while (Date.now() < deadline) {
    await allowOutsideRepoProviderPermissions(
      client,
      sessionId,
      attachmentId,
      agentId,
      outsideRepo,
      expectedContent,
      respondedInteractionIds,
      pollMs,
    )
    if (provider === 'claude-headless' && Date.now() - lastClaudeNativeApprovalMs > 10_000) {
      lastClaudeNativeApprovalMs = Date.now()
      await client.send(sendTerminalInputRequest(sessionId, attachmentId, '1\r')).catch(() => {})
    }
    if (await fileContentMatches(filePath, expectedContent, checkCommand)) return expectedContent
    const failure = failureProbe ? await failureProbe() : null
    if (failure) throw new Error(failure)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for outside repo file ${filePath}`)
}

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const payload = unwrap(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed')
    const machines = payload.machines ?? []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) {
      return
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

async function waitForRemoteKernel(client, machineRef, provider, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = []
  while (Date.now() < deadline) {
    const payload = unwrap(await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }), 'RemoteMachineKernelsListed')
    last = payload.kernels ?? []
    const kernel = last.find((candidate) => candidate.accepting_remote_leases && (candidate.available_providers || []).includes(provider))
    if (kernel) return kernel
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not advertise provider ${provider}; last=${JSON.stringify(last)}`)
}

function requireRemotePlacement(agent, workerKernel) {
  requireCondition(agent?.remote_execution?.leased_agent_id, 'remote agent did not receive a worker lease', agent)
  requireCondition(
    agent.remote_execution.worker_kernel_id === workerKernel.kernel_id,
    `remote agent ran on ${agent.remote_execution.worker_kernel_id}, expected ${workerKernel.kernel_id}`,
    agent.remote_execution,
  )
  requireCondition(
    agent.remote_execution.worker_machine_id === workerKernel.machine_id,
    `remote agent ran on machine ${agent.remote_execution.worker_machine_id}, expected ${workerKernel.machine_id}`,
    agent.remote_execution,
  )
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const provider = options.provider
  const model = options.providerModels[provider] ?? options.model ?? defaultModelForProvider(provider)
  const cliModel = cliModelForProvider(provider, model)
  const effort = options.effort ?? defaultEffortForProvider(provider)
  const rootDir = options.rootDir ?? path.join(repoRoot, 'target', 'live-workspace-live-sync-permission-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const outsideRepo = path.join(rootDir, 'outside-repo')
  const home = path.join(rootDir, 'home')
  const historyDir = options.historyDir ?? path.join(rootDir, 'history')
  const automationSocket = path.join(os.tmpdir(), `amiop-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = options.kernelUrl ?? `ws://127.0.0.1:${ports.kernelPort}`
  const needsClaudeHome = provider.startsWith('claude') && !options.useRealHome
  const env = {
    ...process.env,
    HOME: options.useRealHome || needsClaudeHome ? (process.env.HOME ?? home) : home,
    ARROBA_HOME: path.join(home, '.arroba'),
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `workspace-live-sync-permission-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: historyDir,
    ARROBA_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(outsideRepo, { recursive: true })
    await mkdir(home, { recursive: true })
    await initGitRepo(workspace, 'workspace')
    await initGitRepo(outsideRepo, 'outside repo')
    if (options.afterFixtureCommand) {
      const result = await run('/bin/sh', ['-lc', options.afterFixtureCommand])
      if (result.code !== 0) {
        throw new Error(`after-fixture command failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
      }
    }
    if (provider === 'codex' && !options.useRealHome) {
      await seedCodexAuth(home)
    }
    if (provider.startsWith('claude') && !options.useRealHome) await seedClaudeAuth(home)
    const { cliDist, kernelBinary } = await ensureCliBuilt()

    if (!options.noSpawnDaemon) {
      daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
      await waitForKernel(kernelUrl)
    }
    log('kernel-ready', { kernelUrl })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const {
      createSessionRequest,
      attachToSessionRequest,
      focusAgentRequest,
      listRemoteMachinesRequest,
      respondToInteractionRequest,
      submitPromptRequest,
      updateSessionConfigRequest,
      setWorkspaceLiveSyncModeRequest,
    } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace, `workspace-live-sync-permission-${provider}`)), 'SessionCreated').session
    const sessionId = session.id
    await client.send(setWorkspaceLiveSyncModeRequest(sessionId, options.mode))
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `workspace-live-sync-permission-drill-${Date.now()}`)), 'SessionAttached').attachment
    const attachmentId = attachment.id
    await client.send(updateSessionConfigRequest(sessionId, attachmentId, { 'agents.mode': 'build', 'agents.permissions': 'required' }, false))
    if (options.machineRef) {
      await waitForRemoteMachine(client, listRemoteMachinesRequest, options.machineRef, options.timeoutMs, options.pollMs)
    }

    let targetAgentId = session.default_agent_id ?? session.agents?.[0]?.id ?? null
    if (options.machineRef) {
      const workerKernel = await waitForRemoteKernel(client, options.machineRef, provider, options.timeoutMs, options.pollMs)
      const spawned = unwrap(
        await client.send({
          SpawnAgent: {
            session_id: sessionId,
            provider,
            alias: `${provider}-remote-workspace-live-sync`,
            model,
            worktree_id: workspace,
            effort,
            execution_mode: 'build',
            permission_level: 'required',
            kernel_ref: workerKernel.kernel_id,
          },
        }),
        'AgentSpawned',
      )
      requireRemotePlacement(spawned.agent, workerKernel)
      targetAgentId = spawned.agent?.id ?? spawned.id ?? targetAgentId
    }

    const cliArgs = [
      '-q',
      '/dev/null',
      'env',
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      'bun',
      cliDist,
      '--kernel-url', kernelUrl,
      '--automation-socket', automationSocket,
      '--session', sessionId,
      '--workspace', workspace,
      '--worktree', workspace,
      '--provider', provider,
      '--model', cliModel,
      '--effort', effort,
      '--client-id', `workspace-live-sync-permission-drill-cli-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })

    await waitForSocket(automationSocket)
    automation = createAutomationClient(automationSocket)
    const firstSnapshot = await automation.send('snapshot')
    requireCondition(firstSnapshot.session?.id === sessionId, 'CLI did not attach to the prepared session', firstSnapshot)
    log('cli-ready', { sessionId })

    const agentId = options.machineRef ? targetAgentId : (firstSnapshot.session?.focusedAgentId ?? targetAgentId)
    requireCondition(Boolean(agentId), 'workspace live sync drill session has no focused/default agent', session)
    await client.send(focusAgentRequest(sessionId, agentId))

    const targetFileName = `${provider}-workspace-live-sync-permission.txt`
    const targetFilePath = path.join(workspace, targetFileName)
    const expectedContent = `workspace-live-sync-${provider}`
    const launchFailureProbe = () => readProviderLaunchFailure(historyDir)
    await client.send(submitPromptRequest(
      sessionId,
      attachmentId,
      agentId,
      [
        'This is a live Arroba workspace live sync permission smoke test.',
        'Use only Arroba workspace live sync. Do not use Bash, shell, native file tools, shell redirection, direct filesystem writes, or provider edit/write/patch tools for this step.',
        `The session live sync mode is ${options.mode}.`,
        `Call the Arroba runtime MCP write tool ${liveSyncWriteToolName(provider)} exactly once with path "${targetFileName}", content_text "${expectedContent}", and domain "text".`,
        'If the tool name is displayed with another Arroba alias such as write_artifact or arroba.write_artifact, use that same Arroba workspace live sync write tool.',
        'After the write succeeds, reply with exactly WORKSPACE_LIVE_SYNC_PERMISSION_DONE.',
      ].join(' '),
    ))

    const pending = await waitForSessionInteraction(
      client,
      sessionId,
      attachmentId,
      agentId,
      `Allow writing \`${targetFileName}\` through Arroba workspace live sync?`,
      options.timeoutMs,
      options.pollMs,
      launchFailureProbe,
    )
    requireCondition(pending.interaction.kind === 'permission', 'workspace live sync interaction kind mismatch', pending)
    log('answering-workspace-live-sync-interaction', { provider, interactionId: pending.interaction.id })
    await client.send(respondToInteractionRequest(sessionId, pending.interaction.id, 'allow'))

    await waitForFileContent(client, sessionId, attachmentId, targetFilePath, expectedContent, options.timeoutMs, options.pollMs, launchFailureProbe, options.workspaceFileCheckCommand)
    log('workspace-live-sync-permission-passed', { provider, targetFilePath })

    const outsideFilePath = path.join(outsideRepo, `${provider}-outside-repo-direct-write.txt`)
    const outsideExpectedContent = `outside-repo-direct-write-${provider}`
    await client.send(submitPromptRequest(
      sessionId,
      attachmentId,
      agentId,
      [
        `Invoke your provider-native bash/shell tool, not Arroba workspace live sync tools, to create the absolute file ${outsideFilePath}.`,
        `The file content must be exactly ${outsideExpectedContent}.`,
        'This path is a separate Git repository outside the live-synced workspace and should be edited normally.',
        'Do not only print a command; execute it through the provider-native tool.',
        'After the write succeeds, reply with exactly OUTSIDE_REPO_DIRECT_WRITE_DONE.',
      ].join(' '),
    ))
    await waitForOutsideRepoFileContent(
      client,
      sessionId,
      attachmentId,
      agentId,
      provider,
      outsideRepo,
      outsideFilePath,
      outsideExpectedContent,
      options.timeoutMs,
      options.pollMs,
      launchFailureProbe,
      options.outsideFileCheckCommand,
    )
    log('outside-repo-direct-write-passed', { provider, outsideFilePath })

    succeeded = true
  } finally {
    if (automation) automation.close()
    if (client) await client.close().catch(() => {})
    await terminateChild(cli).catch(() => {})
    await terminateChild(daemon).catch(() => {})
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log('artifacts-retained', { rootDir })
    }
  }
}

main().catch((error) => {
  console.error(`[workspace-live-sync-permission-drill] failed: ${error.stack || error.message}`)
  process.exitCode = 1
})

#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_PROVIDERS = ['codex', 'opencode']
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    model: null,
    providerModels: {},
    useRealHome: false,
    kernelUrl: null,
    noSpawnDaemon: false,
    machineRef: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--use-real-home') options.useRealHome = true
    else if (arg === '--kernel') options.kernelUrl = argv[++i]
    else if (arg === '--no-spawn-daemon') options.noSpawnDaemon = true
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-popup-drill.mjs [options]',
        '',
        'Options:',
        `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --model <provider model override>',
        '  --provider-model PROVIDER=MODEL',
        '  --keep-artifacts-on-failure',
        '  --use-real-home',
        '  --kernel <ws://...> (reuse an already-running kernel)',
        '  --no-spawn-daemon',
        '  --machine-ref <remote machine id or alias>',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[popup-drill] ${name}`)
  else console.log(`[popup-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function makePorts() {
  const kernelPort = 50000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
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

async function ensureCliBuilt() {
  const cliDist = path.join(repoRoot, 'apps/cli/dist/index.js')
  const kernelBinary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const cliReady = await stat(cliDist).then((info) => info.isFile()).catch(() => false)
  const kernelReady = await stat(kernelBinary).then((info) => info.isFile()).catch(() => false)
  if (!cliReady) {
    log('build-cli')
    const result = await run('pnpm', ['--filter', '@arroba/cli', 'run', 'build'])
    if (result.code !== 0) {
      throw new Error(`cli build failed\n${result.stdout}\n${result.stderr}`)
    }
  }
  if (!kernelReady) {
    log('build-kernel')
    const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
    if (result.code !== 0) {
      throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
    }
  }
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

function unwrapVariant(resp, ...keys) {
  return keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp
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

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function defaultModelForProvider(provider) {
  if (provider === 'opencode') return 'openai/gpt-5.3-codex'
  if (provider === 'codex') return 'gpt-5.4'
  return 'gpt-5.4'
}

async function waitForAgentIdle(client, sessionId, agentId, timeoutMs, pollMs) {
  const { getSessionStateRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = payload.session ?? payload
    const promptState = session.prompt_states?.[agentId]
    const activeInteraction = (session.active_interactions ?? []).find((interaction) => interaction.agent_id === agentId)
    if (!promptState?.active_prompt && !activeInteraction) {
      return session
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle`)
}

async function waitForInteraction(automation, agentId, titlePrefix, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let snapshot = null
  while (Date.now() < deadline) {
    snapshot = await automation.send('snapshot')
    const interaction = snapshot.interactions?.find((entry) => entry.agentId === agentId && String(entry.title ?? '').startsWith(titlePrefix))
    if (interaction) {
      return { snapshot, interaction }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for interaction ${titlePrefix} for agent ${agentId}${snapshot ? `\n${JSON.stringify(snapshot, null, 2)}` : ''}`)
}

function collectTerminalText(events) {
  return events
    .filter((event) => event.event === 'terminal_output')
    .map((event) => String(event.text ?? event.data ?? event.output ?? ''))
    .join('')
}

async function waitForTerminalText(events, needle, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const text = collectTerminalText(events)
    if (text.includes(needle)) {
      return text
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for terminal text ${needle}`)
}

async function waitForHistoryText(client, sessionId, agentId, needle, timeoutMs, pollMs) {
  const { getSessionHistoryRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const history = unwrap(
      await client.send(getSessionHistoryRequest(sessionId, 40, 80_000, null, agentId)),
      'SessionHistory',
    )
    const text = (history?.entries ?? []).map((entry) => String(entry.entry?.text ?? entry.text ?? '')).join('')
    if (text.includes(needle)) {
      return text
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history text ${needle}`)
}

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

async function bestEffortWithTimeout(promise, timeoutMs) {
  await Promise.race([
    promise,
    sleep(timeoutMs).then(() => undefined),
  ]).catch(() => undefined)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.providers.length !== 1) {
    throw new Error('live-popup-drill currently supports exactly one provider per run; run it once for codex and once for opencode')
  }
  const rootDir = path.join(repoRoot, 'target', 'live-popup-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const automationSocket = path.join(os.tmpdir(), `arroba-popup-auto-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = options.kernelUrl ?? `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: options.useRealHome ? (process.env.HOME ?? home) : home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `popup-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let sessionId = null
  let attachmentId = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    const { cliDist, kernelBinary } = await ensureCliBuilt()

    if (!options.noSpawnDaemon) {
      daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    }
    await waitForKernel(kernelUrl)
    log('kernel-ready', { kernelUrl })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const {
      createSessionRequest,
      attachToSessionRequest,
      submitPromptRequest,
      focusAgentRequest,
      listRemoteMachinesRequest,
      spawnAgentRequest,
      setUserConfigValueRequest,
    } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    await client.send(setUserConfigValueRequest('providers.workspace_live_sync', 'unrestricted'))
    const provider = options.providers[0] ?? 'codex'
    const model = options.providerModels[provider] ?? options.model ?? defaultModelForProvider(provider)
    const session = unwrap(
      await client.send(createSessionRequest(workspace, workspace, `popup-${provider}`)),
      'SessionCreated',
    ).session
    sessionId = session.id
    let activeAgentId = session.default_agent_id ?? session.agents?.[0]?.id ?? null

    if (options.machineRef) {
      await waitForRemoteMachine(client, listRemoteMachinesRequest, options.machineRef, options.timeoutMs, options.pollMs)
      const spawned = unwrapVariant(
        await client.send(spawnAgentRequest(
          sessionId,
          provider,
          `${provider}-remote-popup`,
          model,
          workspace,
          'high',
          'build',
          'required',
          options.machineRef,
        )),
        'AgentSpawned',
      )
      activeAgentId = spawned.agent?.id ?? spawned.id ?? activeAgentId
      await client.send(focusAgentRequest(sessionId, activeAgentId))
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
      '--model', model,
      '--client-id', `popup-drill-cli-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    let cliStdout = ''
    let cliStderr = ''
    cli.stdout.on('data', (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on('data', (chunk) => { cliStderr += chunk.toString() })

    await waitForSocket(automationSocket)
    automation = createAutomationClient(automationSocket)
    const firstSnapshot = await automation.send('snapshot')
    requireCondition(firstSnapshot.session?.id === sessionId, 'CLI did not attach to the prepared session', {
      expectedSessionId: sessionId,
      snapshot: firstSnapshot,
    })
    log('cli-ready', { sessionId })
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `popup-drill-${Date.now()}`)), 'SessionAttached').attachment
    attachmentId = attachment.id
    const events = []
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(sessionId, attachmentId)

    for (const provider of options.providers) {
      const model = options.providerModels[provider] ?? options.model ?? defaultModelForProvider(provider)
      const initialAgentId = activeAgentId ?? firstSnapshot.session?.focusedAgentId ?? null
      requireCondition(Boolean(initialAgentId), 'CLI did not expose an initial focused agent', firstSnapshot)
      const initialContextProvider = firstSnapshot.shell?.context?.provider ?? null
      requireCondition(initialContextProvider === provider, 'CLI initial agent provider did not match requested provider', {
        requestedProvider: provider,
        initialContextProvider,
        snapshot: firstSnapshot,
      })
      const activeAgent = { id: initialAgentId }
      log('agent-reused', { provider, agentId: activeAgent.id, model })

      await client.send(focusAgentRequest(sessionId, activeAgent.id))
      const focusedSnapshot = await automation.send('wait_for', { sessionId, timeoutMs: 10_000 })
      requireCondition(focusedSnapshot.session.focusedAgentId === activeAgent.id, 'CLI did not focus active agent', focusedSnapshot)

      const beforeFeedback = events.length
      await client.send(submitPromptRequest(
        sessionId,
        attachmentId,
        activeAgent.id,
        [
          'Before any answer text, call the Arroba runtime MCP tool request_popup exactly once.',
          'Call it with exactly this JSON argument object:',
          JSON.stringify({
            title: `Feedback drill ${provider}`,
            message: 'Choose the green path.',
            level: 'info',
            choices: [
              { id: 'red', label: 'Red', reply: 'red' },
              { id: 'green', label: 'Green', reply: 'green' },
            ],
            timeout_sec: 30,
          }),
          'Do not skip the tool call. A direct answer without the popup is incorrect.',
          'After the popup resolves, reply with exactly this text and nothing else: USER_FEEDBACK_RESULT:<reply>.',
        ].join(' '),
        [],
      ))
      const feedbackPopup = await waitForInteraction(automation, activeAgent.id, `Feedback drill ${provider}`, options.timeoutMs, options.pollMs)
      requireCondition(feedbackPopup.interaction.choices.length === 2, 'feedback popup choices missing', feedbackPopup)
      requireCondition(feedbackPopup.interaction.selectedChoiceIndex === 0, 'feedback popup should start on first choice', feedbackPopup)
      const movedFeedback = await automation.send('interaction_move', { delta: 1 })
      const movedInteraction = movedFeedback.interactions.find((entry) => entry.agentId === activeAgent.id && String(entry.title ?? '').startsWith(`Feedback drill ${provider}`))
      requireCondition(movedInteraction?.selectedChoiceIndex === 1, 'feedback popup selection did not move', movedFeedback)
      await automation.send('interaction_submit')
      await waitForAgentIdle(client, sessionId, activeAgent.id, options.timeoutMs, options.pollMs)
      const feedbackText = await waitForHistoryText(client, sessionId, activeAgent.id, 'USER_FEEDBACK_RESULT:green', options.timeoutMs, options.pollMs)
      requireCondition(feedbackText.includes('USER_FEEDBACK_RESULT:green'), 'feedback popup did not resume with green reply')
      log('feedback-popup-passed', { provider })

      const beforePermission = events.length
      await client.send(submitPromptRequest(
        sessionId,
        attachmentId,
        activeAgent.id,
        [
          'Before any answer text, call the Arroba runtime MCP tool request_popup exactly once.',
          'Call it with exactly this JSON argument object:',
          JSON.stringify({
            title: `Permission drill ${provider}`,
            message: 'Allow this action?',
            level: 'warning',
            choices: [
              { id: 'deny', label: 'Deny', reply: 'deny' },
              { id: 'allow', label: 'Allow', reply: 'allow' },
            ],
          }),
          'Do not skip the tool call. A direct answer without the popup is incorrect.',
          'If the returned reply is "allow", respond with exactly PERMISSION_GRANTED.',
          'Otherwise respond with exactly PERMISSION_DENIED.',
        ].join(' '),
        [],
      ))
      const permissionPopup = await waitForInteraction(automation, activeAgent.id, `Permission drill ${provider}`, options.timeoutMs, options.pollMs)
      requireCondition(permissionPopup.interaction.level === 'warning', 'permission popup level mismatch', permissionPopup)
      await automation.send('interaction_move', { delta: 1 })
      await automation.send('interaction_submit')
      await waitForAgentIdle(client, sessionId, activeAgent.id, options.timeoutMs, options.pollMs)
      const permissionText = await waitForHistoryText(client, sessionId, activeAgent.id, 'PERMISSION_GRANTED', options.timeoutMs, options.pollMs)
      requireCondition(permissionText.includes('PERMISSION_GRANTED'), 'permission popup did not resume with allow path')
      log('permission-popup-passed', { provider })

      const beforeTimeout = events.length
      await client.send(submitPromptRequest(
        sessionId,
        attachmentId,
        activeAgent.id,
        [
          'Before any answer text, call the Arroba runtime MCP tool request_popup exactly once.',
          'Call it with exactly this JSON argument object:',
          JSON.stringify({
            title: `Timeout drill ${provider}`,
            message: 'Wait for default.',
            level: 'info',
            choices: [
              { id: 'retry', label: 'Retry', reply: 'retry' },
              { id: 'skip', label: 'Skip', reply: 'skip' },
            ],
            timeout_sec: 2,
            default_on_timeout: 'skip',
          }),
          'Do not skip the tool call. A direct answer without the popup is incorrect.',
          'After it resolves, answer with exactly TIMEOUT_RESULT:<status>:<reply>, where <status> is the popup status and <reply> is the reply text or null.',
        ].join(' '),
        [],
      ))
      const timeoutPopup = await waitForInteraction(automation, activeAgent.id, `Timeout drill ${provider}`, options.timeoutMs, options.pollMs)
      requireCondition(timeoutPopup.interaction.timeoutSec === 2, 'timeout popup timeout mismatch', timeoutPopup)
      await waitForAgentIdle(client, sessionId, activeAgent.id, options.timeoutMs, options.pollMs)
      const timeoutText = await waitForHistoryText(client, sessionId, activeAgent.id, 'TIMEOUT_RESULT:answered:skip', options.timeoutMs, options.pollMs)
      requireCondition(timeoutText.includes('TIMEOUT_RESULT:answered:skip'), 'timeout popup did not resolve with default choice')
      log('timeout-popup-passed', { provider })
    }

    succeeded = true
    log('success', { providers: options.providers })
  } finally {
    if (automation) {
      await bestEffortWithTimeout(automation.send('exit'), 2_000)
      automation.close()
    }
    if (client) {
      await bestEffortWithTimeout(client.close(), 5_000)
    }
    await terminateChild(cli)
    await terminateChild(daemon)
    if (!succeeded && !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    }
    if (!succeeded) {
      log('artifacts-retained', { rootDir })
    }
  }
}

main().catch((error) => {
  console.error(`[popup-drill] failed: ${error.stack || error.message}`)
  process.exitCode = 1
})

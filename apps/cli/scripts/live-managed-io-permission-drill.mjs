#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, stat, readFile } from 'node:fs/promises'
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
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--use-real-home') options.useRealHome = true
    else if (arg === '--kernel') options.kernelUrl = argv[++i]
    else if (arg === '--no-spawn-daemon') options.noSpawnDaemon = true
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-managed-io-permission-drill.mjs [options]',
        '',
        'Options:',
        '  --provider <codex|opencode>',
        '  --model <provider model override>',
        '  --kernel <ws://...> (reuse an already-running kernel)',
        '  --no-spawn-daemon',
        '  --machine-ref <remote machine id or alias>',
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
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[managed-io-permission-drill] ${name}`)
  else console.log(`[managed-io-permission-drill] ${name}`, JSON.stringify(details))
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
    const result = await run('pnpm', ['--filter', '@arroba/cli', 'run', 'build'])
    if (result.code !== 0) throw new Error(`cli build failed\n${result.stdout}\n${result.stderr}`)
  }
  if (!kernelReady) {
    const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
    if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
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

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2000)])
  }
}

async function waitForSessionInteraction(client, sessionId, agentId, containsText, timeoutMs, pollMs) {
  const { getSessionStateRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + timeoutMs
  let session = null
  while (Date.now() < deadline) {
    const response = await client.send(getSessionStateRequest(sessionId))
    const payload = response?.SessionStateLoaded ?? response?.SessionState ?? response
    session = payload.session ?? payload
    const interaction = (session.active_interactions ?? []).find((entry) => entry.agent_id === agentId && String(entry.message ?? '').includes(containsText))
      ?? (session.active_interactions ?? []).find((entry) => String(entry.message ?? '').includes(containsText))
    if (interaction) return { session, interaction }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for session interaction containing ${containsText}${session ? `\n${JSON.stringify(session, null, 2)}` : ''}`)
}

async function waitForFileContent(filePath, expectedContent, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const content = await readFile(filePath, 'utf8')
      if (content.trim() === expectedContent) return content
    } catch {}
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for file ${filePath}`)
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const provider = options.provider
  const model = options.model ?? defaultModelForProvider(provider)
  const rootDir = path.join(repoRoot, 'target', 'live-managed-io-permission-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const automationSocket = path.join(os.tmpdir(), `amiop-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = options.kernelUrl ?? `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: options.useRealHome ? (process.env.HOME ?? home) : home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `managed-io-permission-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
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
      spawnAgentRequest,
      submitPromptRequest,
      updateSessionConfigRequest,
      setUserConfigValueRequest,
    } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    await client.send(setUserConfigValueRequest('providers.managed_io', 'required'))

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace, `managed-io-permission-${provider}`)), 'SessionCreated').session
    const sessionId = session.id
    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `managed-io-permission-drill-${Date.now()}`)), 'SessionAttached').attachment
    const attachmentId = attachment.id
    await client.send(updateSessionConfigRequest(sessionId, attachmentId, { 'agents.mode': 'build', 'agents.permissions': 'required' }, false))
    if (options.machineRef) {
      await waitForRemoteMachine(client, listRemoteMachinesRequest, options.machineRef, options.timeoutMs, options.pollMs)
    }

    let targetAgentId = session.default_agent_id ?? session.agents?.[0]?.id ?? null
    if (options.machineRef) {
      const spawned = unwrap(
        await client.send(spawnAgentRequest(
          sessionId,
          provider,
          `${provider}-remote-managed-io`,
          model,
          workspace,
          'high',
          'build',
          'required',
          options.machineRef,
        )),
        'AgentSpawned',
      )
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
      '--model', model,
      '--client-id', `managed-io-permission-drill-cli-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })

    await waitForSocket(automationSocket)
    automation = createAutomationClient(automationSocket)
    const firstSnapshot = await automation.send('snapshot')
    requireCondition(firstSnapshot.session?.id === sessionId, 'CLI did not attach to the prepared session', firstSnapshot)
    log('cli-ready', { sessionId })

    const agentId = options.machineRef ? targetAgentId : (firstSnapshot.session?.focusedAgentId ?? targetAgentId)
    requireCondition(Boolean(agentId), 'managed I/O drill session has no focused/default agent', session)
    await client.send(focusAgentRequest(sessionId, agentId))

    const targetFileName = `${provider}-managed-io-permission.txt`
    const targetFilePath = path.join(workspace, targetFileName)
    const expectedContent = `managed-io-${provider}`
    await client.send(submitPromptRequest(
      sessionId,
      attachmentId,
      agentId,
      [
        `Use Arroba managed I/O tool write_artifact to create a file named ${targetFileName} with the exact text ${expectedContent}.`,
        'Do not use shell redirection or direct filesystem writes.',
        'After the write succeeds, reply with exactly MANAGED_IO_PERMISSION_DONE.',
      ].join(' '),
    ))

    const pending = await waitForSessionInteraction(
      client,
      sessionId,
      agentId,
      `Allow writing \`${targetFileName}\` through Arroba managed I/O?`,
      options.timeoutMs,
      options.pollMs,
    )
    requireCondition(pending.interaction.kind === 'permission', 'managed I/O interaction kind mismatch', pending)
    log('answering-managed-io-interaction', { provider, interactionId: pending.interaction.id })
    await client.send(respondToInteractionRequest(sessionId, pending.interaction.id, 'allow'))

    await waitForFileContent(targetFilePath, expectedContent, options.timeoutMs, options.pollMs)
    log('managed-io-permission-passed', { provider, targetFilePath })

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
  console.error(`[managed-io-permission-drill] failed: ${error.stack || error.message}`)
  process.exitCode = 1
})

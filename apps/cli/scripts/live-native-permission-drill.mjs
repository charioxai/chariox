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
    else if (arg === '--kernel') options.kernelUrl = argv[++i]
    else if (arg === '--no-spawn-daemon') options.noSpawnDaemon = true
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-native-permission-drill.mjs [options]',
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
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[native-permission-drill] ${name}`)
  else console.log(`[native-permission-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function bestEffortWithTimeout(promise, timeoutMs) {
  await Promise.race([
    promise,
    sleep(timeoutMs).then(() => undefined),
  ]).catch(() => undefined)
}

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

function makePorts() {
  const kernelPort = 51000 + Math.floor(Math.random() * 1000)
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

function unwrapVariant(resp, ...keys) {
  return keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp
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
  const needles = Array.isArray(containsText) ? containsText : [containsText]
  while (Date.now() < deadline) {
    const response = await client.send(getSessionStateRequest(sessionId))
    const payload = response?.SessionStateLoaded ?? response?.SessionState ?? response
    session = payload.session ?? payload
    const matchesNeedle = (entry) => needles.some((needle) => String(entry.message ?? '').includes(needle))
    const interaction = (session.active_interactions ?? []).find((entry) => entry.agent_id === agentId && matchesNeedle(entry))
      ?? (session.active_interactions ?? []).find(matchesNeedle)
    if (interaction) return { session, interaction }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for session interaction containing ${needles.join(' or ')}${session ? `\n${JSON.stringify(session, null, 2)}` : ''}`)
}

function collectTerminalText(events) {
  return events
    .filter((event) => event.event === 'terminal_output')
    .map((event) => {
      if (Array.isArray(event.records)) {
        return event.records
          .map((record) => {
            if (record.kind === 'PromptEcho') return ''
            if (Array.isArray(record.bytes)) {
              return Buffer.from(record.bytes).toString('utf8')
            }
            return String(record.text ?? record.data ?? record.output ?? '')
          })
          .join('')
      }
      return String(event.text ?? event.data ?? event.output ?? '')
    })
    .join('')
}

async function waitForTerminalText(events, needle, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const text = collectTerminalText(events)
    if (text.includes(needle)) return text
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for terminal text ${needle}`)
}

async function waitForFileContent(filePath, expectedContent, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const content = await readFile(filePath, 'utf8')
      if (content.trim() === expectedContent) return content
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for file content ${expectedContent} at ${filePath}: ${lastError?.message ?? 'content mismatch'}`)
}

async function waitForAgentIdle(client, sessionId, agentId, timeoutMs, pollMs) {
  const { getSessionStateRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = payload.session ?? payload
    const promptState = session.prompt_states?.[agentId]
    const activeInteraction = (session.active_interactions ?? []).find((interaction) => interaction.agent_id === agentId)
    if (!promptState?.active_prompt && !activeInteraction) return session
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle`)
}

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed')
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
  const rootDir = path.join(repoRoot, 'target', 'live-native-permission-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const automationSocket = path.join(os.tmpdir(), `anp-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = options.kernelUrl ?? `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: process.env.HOME,
    ARROBA_LOG_DIR: path.join(rootDir, 'logs'),
    ARROBA_LOG_LEVEL: 'debug',
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `native-permission-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let attachmentId = null
  let sessionId = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
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
      getSessionStateRequest,
      listRemoteMachinesRequest,
      respondToInteractionRequest,
      spawnAgentRequest,
      submitPromptRequest,
      focusAgentRequest,
      updateSessionConfigRequest,
      setUserConfigValueRequest,
    } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)

    await client.send(setUserConfigValueRequest('providers.workspace_live_sync', 'unrestricted'))

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace, `native-permission-${provider}`)), 'SessionCreated').session
    sessionId = session.id

    const attachment = unwrap(await client.send(attachToSessionRequest(sessionId, `native-permission-drill-${Date.now()}`)), 'SessionAttached').attachment
    attachmentId = attachment.id
    const events = []
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(sessionId, attachmentId)

    if (options.machineRef) {
      await waitForRemoteMachine(client, listRemoteMachinesRequest, options.machineRef, options.timeoutMs, options.pollMs)
    }

    await client.send(updateSessionConfigRequest(sessionId, attachmentId, { 'agents.mode': 'build', 'agents.permissions': 'required' }, false))
    let defaultAgentId = session.default_agent_id ?? session.agents?.[0]?.id ?? null
    if (options.machineRef) {
      const spawned = unwrapVariant(
        await client.send(spawnAgentRequest(
          sessionId,
          provider,
          `${provider}-remote-native`,
          model,
          workspace,
          'high',
          'build',
          'required',
          options.machineRef,
        )),
        'AgentSpawned',
      )
      defaultAgentId = spawned.agent?.id ?? spawned.id ?? defaultAgentId
    }
    requireCondition(Boolean(defaultAgentId), 'created session did not expose a usable target agent', session)

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
      '--client-id', `native-permission-drill-cli-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })

    await waitForSocket(automationSocket)
    automation = createAutomationClient(automationSocket)
    const firstSnapshot = await automation.send('snapshot')
    requireCondition(firstSnapshot.session?.id === sessionId, 'CLI did not attach to the prepared session', firstSnapshot)
    const sessionStatePayload = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const sessionState = sessionStatePayload.session ?? sessionStatePayload
    const cliAttachmentId = (sessionState.attachment_ids ?? []).find((id) => id !== attachmentId) ?? null
    requireCondition(Boolean(cliAttachmentId), 'session state did not expose a distinct CLI attachment id', {
      attachmentIds: sessionState.attachment_ids,
      helperAttachmentId: attachmentId,
      snapshot: firstSnapshot,
    })

    const activeAgentId = options.machineRef ? defaultAgentId : (firstSnapshot.session?.focusedAgentId ?? defaultAgentId)
    requireCondition(Boolean(activeAgentId), 'CLI did not expose focused agent', firstSnapshot)
    await client.send(focusAgentRequest(sessionId, activeAgentId))

    const bashNeedle = provider === 'codex' ? 'Approve command execution?' : 'Approve OpenCode bash request?'
    const editNeedle = provider === 'codex' ? ['Approve file changes?', 'Approve command execution?'] : 'Approve OpenCode edit request?'
    const codexSandboxEscapePath = `/tmp/arroba-codex-native-bash-${process.pid}.txt`
    const bashPrompt = provider === 'codex'
      ? `Use the shell to run \`printf 'native-bash\\n' > ${codexSandboxEscapePath}\`. After the command succeeds, reply with exactly NATIVE_BASH_PERMISSION_DONE.`
      : "Use the shell to run `python3 -c \"print('native-bash')\"`. After the command succeeds, reply with exactly NATIVE_BASH_PERMISSION_DONE."

    const beforeBash = events.length
    await client.send(submitPromptRequest(sessionId, attachmentId, activeAgentId, bashPrompt))
    const bashInteraction = await waitForSessionInteraction(client, sessionId, activeAgentId, bashNeedle, options.timeoutMs, options.pollMs)
    requireCondition(bashInteraction.interaction.level === 'warning' || bashInteraction.interaction.level === 'critical', 'unexpected bash interaction level', bashInteraction)
    log('answering-bash-interaction', {
      provider,
      interactionId: bashInteraction.interaction.id,
      level: bashInteraction.interaction.level,
      message: bashInteraction.interaction.message,
    })
    await client.send(respondToInteractionRequest(sessionId, bashInteraction.interaction.id, 'allow_once'))
    await waitForAgentIdle(client, sessionId, activeAgentId, options.timeoutMs, options.pollMs)
    await waitForTerminalText(events.slice(beforeBash), 'NATIVE_BASH_PERMISSION_DONE', options.timeoutMs, options.pollMs)
    log('bash-permission-passed', { provider })

    const beforeEdit = events.length
    const createdFile = path.join(workspace, `${provider}-native-permission.txt`)
    await client.send(submitPromptRequest(
      sessionId,
      attachmentId,
      activeAgentId,
      `Create a file named ${path.basename(createdFile)} with the exact text native-${provider}. After the write succeeds, reply with exactly NATIVE_EDIT_PERMISSION_DONE.`,
    ))
    const editInteraction = await waitForSessionInteraction(client, sessionId, activeAgentId, editNeedle, options.timeoutMs, options.pollMs)
    requireCondition(editInteraction.interaction.level === 'critical' || editInteraction.interaction.level === 'warning', 'unexpected edit interaction level', editInteraction)
    log('answering-edit-interaction', {
      provider,
      interactionId: editInteraction.interaction.id,
      level: editInteraction.interaction.level,
      message: editInteraction.interaction.message,
    })
    await client.send(respondToInteractionRequest(sessionId, editInteraction.interaction.id, 'allow_once'))
    await waitForAgentIdle(client, sessionId, activeAgentId, options.timeoutMs, options.pollMs)
    const content = await waitForFileContent(createdFile, `native-${provider}`, options.timeoutMs, options.pollMs)
    requireCondition(content.trim() === `native-${provider}`, 'provider did not create expected file content', { provider, createdFile, content })
    log('edit-permission-passed', { provider, createdFile })

    succeeded = true
    log('success', { provider, model })
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
    if (!succeeded && options.keepArtifactsOnFailure) {
      log('artifacts-kept', { rootDir })
    } else {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    }
  }
}

main().catch((error) => {
  console.error(error.stack || error.message || String(error))
  process.exit(1)
})

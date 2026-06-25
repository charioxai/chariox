#!/usr/bin/env node
import http from 'node:http'
import { spawn } from 'node:child_process'
import { access, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const artifactsDir = path.join(repoRoot, '.artifacts')

const DEFAULT_PROVIDERS = ['codex', 'claude-p', 'claude-headless', 'pi']
const DEFAULT_MODEL = 'gpt-5.5'
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

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
    if (arg === '--') continue
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') {
      console.log([
        'Usage: node apps/cli/scripts/live-agent-vault-credential-drill.mjs [options]',
        '',
        'Prompts live Arroba agents to create generated and user-entered vault credentials,',
        'then use those handles through arroba.http_request_with_credential.',
        '',
        'Options:',
        `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
        `  --model ${DEFAULT_MODEL}`,
        '  --provider-model PROVIDER=MODEL',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (!options.providers.length) throw new Error('at least one provider is required')
  return options
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'claude' || provider === 'claude-p' || provider === 'claude-headless') return 'sonnet'
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  if (provider === 'pi' && !options.model.includes('/')) return `pi/openai-codex/${options.model}`
  return options.model
}

async function run(command, args, options = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (error) => resolve({ code: -1, stdout, stderr: `${stderr}${error.message}` }))
    child.on('close', (code, signal) => resolve({ code: code ?? -1, signal, stdout, stderr }))
  })
}

async function resolveKernelBinary() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', [
    'build',
    '--manifest-path',
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    '--bin',
    'arroba-kernel',
  ])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  await access(binary)
  return binary
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
  child.kill('SIGTERM')
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2_000)])
  }
}

function makePorts() {
  const base = 62000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  let lastError = null
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const created = unwrapVariant(
        await probe.send(createSessionRequest(workspace, workspace, `m17-vault-probe-${attempt}`)),
        'SessionCreated',
      )
      await probe.send(endSessionRequest(created.session.id)).catch(() => {})
      await probe.close()
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForActiveProviderRun({ client, sessionId, providerRunId, getSessionStateRequest, getProviderRunRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const runResponse = await client.send(getProviderRunRequest(providerRunId)).catch(() => null)
    const run = unwrapVariant(runResponse, 'ProviderRun')?.provider_run
    const runState = String(run?.state ?? '').toLowerCase()
    if (runState === 'ended' || runState === 'failed' || runState === 'error') {
      throw new Error(`provider run ${providerRunId} ended before becoming active`)
    }
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    if (session.active_provider_run_id === providerRunId) return run
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId}`)
}

async function launchProviderRunAndWait({
  client,
  sessionId,
  agentId,
  provider,
  model,
  launchProviderRunRequest,
  getSessionStateRequest,
  getProviderRunRequest,
  timeoutMs,
  pollMs,
}) {
  const accepted = unwrapVariant(
    await client.send(launchProviderRunRequest(sessionId, provider, 'default', model, 'low', agentId)),
    'ProviderRunLaunchAccepted',
    'ProviderRunLaunched',
  )
  const run = accepted.provider_run
  await waitForActiveProviderRun({
    client,
    sessionId,
    providerRunId: run.id,
    getSessionStateRequest,
    getProviderRunRequest,
    timeoutMs: Math.min(timeoutMs, 120_000),
    pollMs,
  })
  return run
}

async function waitForAgentPromptIdle({ client, sessionId, attachmentId, agentId, getSessionStateRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const promptState = session.prompt_states?.[agentId] ?? {}
    const agent = (session.agents ?? []).find((candidate) => candidate.id === agentId)
    const activePrompt = promptState.active_prompt ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    const queuedPrompts = promptState.queued_prompts ?? (session.queued_prompts ?? []).filter((prompt) => prompt.target_agent_id === agentId)
    if (!activePrompt && queuedPrompts.length === 0 && agent && !agent.is_processing && agent.state !== 'Working') return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle`)
}

function toolName(update) {
  return String(update?.tool ?? '').toLowerCase()
}

async function waitForHistoryToolCall({ historyDir, agentId, sinceMs, timeoutMs, pollMs, predicate }) {
  const started = Date.now()
  const toolStarts = new Map()
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
        if ((entry.timestamp_ms ?? 0) < sinceMs || entry.agent_id !== agentId) continue
        if (entry.kind === 'provider_error') {
          throw new Error(`provider error while waiting for tool call: ${entry.text}`)
        }
        if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
        let update
        try {
          update = JSON.parse(entry.text)
        } catch {
          continue
        }
        const toolKey = entry.merge_key ?? update.id
        if (update.status === 'started' && update.input !== undefined) {
          toolStarts.set(toolKey, { input: update.input, tool: update.tool })
        } else if (toolStarts.has(toolKey)) {
          const startedUpdate = toolStarts.get(toolKey)
          update = {
            ...update,
            input: update.input ?? startedUpdate.input,
            tool: update.tool === 'tool_result' ? startedUpdate.tool : update.tool,
          }
        }
        if (predicate(update)) return update
      }
    }
    await sleep(pollMs)
  }
  throw new Error('timed out waiting for expected provider tool call in history')
}

async function providerTranscript({ historyDir, agentId, sinceMs }) {
  const entries = []
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
      if ((entry.timestamp_ms ?? 0) < sinceMs || entry.agent_id !== agentId) continue
      entries.push(entry)
    }
  }
  entries.sort((a, b) => (a.timestamp_ms ?? 0) - (b.timestamp_ms ?? 0))
  return entries
}

async function respondToSecretInteraction({ client, sessionId, agentId, secret, getSessionStateRequest, respondToInteractionRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  let observed = null
  while (Date.now() - started < timeoutMs) {
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const interaction = (session.active_interactions ?? [])
      .find((entry) => entry.agent_id === agentId &&
        entry.custom_choice?.input_kind === 'secret' &&
        !String(entry.title ?? '').includes('Unlock Arroba Vault'))
    if (interaction) {
      observed = interaction
      await client.send(respondToInteractionRequest(sessionId, interaction.id, interaction.custom_choice.id, secret))
      return observed
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for secret interaction for ${agentId}`)
}

async function respondToVaultUnlockInteraction({ client, sessionId, agentId, passphrase, choiceId = 'unlock_default_ttl', getSessionStateRequest, respondToInteractionRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const interaction = (session.active_interactions ?? [])
      .find((entry) => entry.agent_id === agentId &&
        entry.custom_choice?.input_kind === 'secret' &&
        String(entry.title ?? '').includes('Unlock Arroba Vault'))
    if (interaction) {
      const choice = (interaction.choices ?? []).find((entry) => entry.id === choiceId)
      if (!choice) {
        throw new Error(`vault unlock interaction did not offer choice ${choiceId}; choices=${JSON.stringify(interaction.choices ?? [])}`)
      }
      await client.send(respondToInteractionRequest(sessionId, interaction.id, choice.id, passphrase))
      return interaction
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for Arroba vault unlock interaction for ${agentId}`)
}

function startCredentialEchoServer(expectedUserSecret) {
  const calls = []
  const server = http.createServer((req, res) => {
    const auth = String(req.headers.authorization ?? '')
    const route = req.url?.split('?')[0] ?? '/'
    const verified = route === '/user'
      ? auth === `Bearer ${expectedUserSecret}`
      : auth.startsWith('Bearer ') && auth.length >= 20
    calls.push({ route, verified, authLength: auth.length })
    const body = JSON.stringify({ route, verified, auth_length: auth.length })
    res.writeHead(200, {
      'content-type': 'application/json',
      'content-length': Buffer.byteLength(body),
      connection: 'close',
    })
    res.end(body)
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      resolve({
        port: address.port,
        calls,
        close: () => new Promise((done) => server.close(done)),
      })
    })
  })
}

async function waitForVerifiedEchoCall({ calls, route, sinceIndex, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const call = calls.slice(sinceIndex).find((candidate) => candidate.route === route && candidate.verified)
    if (call) return call
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for verified credential request to ${route}; observed=${JSON.stringify(calls.slice(sinceIndex))}`)
}

async function renderTerminalScreenshot(fileName, title, lines) {
  await mkdir(artifactsDir, { recursive: true })
  const width = 1280
  const height = Math.max(300, 104 + lines.length * 28)
  const svgPath = path.join(artifactsDir, `${fileName}.svg`)
  const pngPath = path.join(artifactsDir, fileName)
  const escaped = (value) => String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
  const body = lines.map((line, index) => (
    `<text x="48" y="${116 + index * 28}" fill="${line.startsWith('PASS') ? '#8ef0a5' : '#d9e2ec'}" font-size="20">${escaped(line)}</text>`
  )).join('\n')
  await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
<rect width="100%" height="100%" fill="#101820"/>
<rect x="28" y="28" width="${width - 56}" height="${height - 56}" rx="8" fill="#141f2b" stroke="#3b5269"/>
<text x="48" y="72" fill="#ffffff" font-family="Menlo, Consolas, monospace" font-size="24" font-weight="700">${escaped(title)}</text>
<g font-family="Menlo, Consolas, monospace">${body}</g>
</svg>`, 'utf8')
  const result = await run('sips', ['-s', 'format', 'png', svgPath, '--out', pngPath])
  if (result.code !== 0) throw new Error(`failed to render screenshot ${fileName}: ${result.stdout}\n${result.stderr}`)
  await rm(svgPath, { force: true })
  return pngPath
}

function assertNoSecretLeak(entries, secret, label) {
  const payload = JSON.stringify(entries)
  if (payload.includes(secret)) {
    throw new Error(`${label} leaked user-entered secret into provider history`)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-agent-vault-credential-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const historyDir = path.join(rootDir, 'history')
  const capabilityRoot = path.join(rootDir, 'capabilities')
  const arrobaHome = path.join(rootDir, 'arroba-home')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const vaultService = `arroba-m17-drill-${process.pid}-${Date.now()}`
  const vaultPath = path.join(rootDir, 'vault', 'vault.db')
  const vaultPassphrase = `m26-vault-passphrase-${process.pid}-${Date.now()}`
  const daemonEnv = {
    ...process.env,
    ARROBA_HOME: arrobaHome,
    XDG_CONFIG_HOME: path.join(rootDir, 'xdg-config'),
    XDG_STATE_HOME: path.join(rootDir, 'xdg-state'),
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `m17-vault-credential-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: historyDir,
    ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
  }

  const userSecret = `m17-user-secret-${process.pid}-${Date.now()}`

  const results = []
  const credentialKeys = []
  let deleteCredentialSecretRequest = null
  let endSessionRequest = null
  let client = null
  let echo = null
  let daemon = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(arrobaHome, { recursive: true })
    await writeFile(path.join(workspace, 'README.md'), '# M17 agent vault credential live drill\n', 'utf8')
    await mkdir(path.join(daemonEnv.XDG_CONFIG_HOME, 'arroba'), { recursive: true })
    await writeFile(path.join(daemonEnv.XDG_CONFIG_HOME, 'arroba', 'config.toml'), [
      'version = 1',
      '',
      '[credential_vault]',
      `service = "${vaultService}"`,
      `path = "${vaultPath}"`,
      'backend = "arroba_encrypted"',
      'unlock_policy = "ttl"',
      'default_ttl_minutes = 30',
      'max_ttl_minutes = 240',
      'agent_management = "allow"',
      '',
    ].join('\n'), 'utf8')

    const [{ LocalIpcClient }, requests] = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    const {
      attachToSessionRequest,
      createSessionRequest,
      getProviderRunRequest,
      getSessionStateRequest,
      launchProviderRunRequest,
      listProviderProcessesRequest,
      respondToInteractionRequest,
      spawnAgentRequest,
      submitPromptRequest,
      teardownProviderProcessesRequest,
    } = requests
    deleteCredentialSecretRequest = requests.deleteCredentialSecretRequest
    endSessionRequest = requests.endSessionRequest

    const kernel = await resolveKernelBinary()
    echo = await startCredentialEchoServer(userSecret)
    daemon = spawn(kernel, [], {
      cwd: repoRoot,
      env: daemonEnv,
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace)
    client = new LocalIpcClient(kernelUrl)
    const created = unwrapVariant(
      await client.send(createSessionRequest(
        workspace,
        workspace,
        `m17-vault-${Date.now()}`,
        undefined,
        undefined,
        'unrestricted',
      )),
      'SessionCreated',
    )
    const session = created.session
    sessionId = session.id
    const attachment = unwrapVariant(
      await client.send(attachToSessionRequest(session.id, `m17-vault-${Date.now()}`)),
      'SessionAttached',
    ).attachment

    for (const provider of options.providers) {
      const nonce = `${Date.now()}-${Math.floor(Math.random() * 10000)}`
      const generatedCredentialId = `m17-generated-${provider}-${nonce}`
      const userCredentialId = `m17-user-${provider}-${nonce}`
      credentialKeys.push(generatedCredentialId, userCredentialId)

      const agent = unwrapVariant(
        await client.send(spawnAgentRequest(
          session.id,
          provider,
          `${provider}-m17-vault-credential`,
          modelForProvider(provider, options),
          workspace,
          'low',
        )),
        'AgentSpawned',
      ).agent
      await launchProviderRunAndWait({
        client,
        sessionId: session.id,
        agentId: agent.id,
        provider,
        model: modelForProvider(provider, options),
        launchProviderRunRequest,
        getSessionStateRequest,
        getProviderRunRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })

      const generatedStartedAt = Date.now()
      const generatedEchoStart = echo.calls.length
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is an M17/M26 Arroba vault credential creation live drill.',
        'Use Arroba runtime MCP tools only. Do not write files for this task.',
        'Step 1: call `arroba.create_generated_credential` with this exact JSON argument:',
        JSON.stringify({
          credential: {
            id: generatedCredentialId,
            description: `Generated credential drill for ${provider}`,
            allowed_hosts: [`127.0.0.1:${echo.port}`],
            allowed_uses: ['http'],
            injection: { kind: 'header', name: 'authorization', value: 'Bearer ${secret}' },
          },
          generator: { kind: 'password', length: 24, symbols: true, avoid_ambiguous: true },
          overwrite: false,
        }),
        'Step 2: call `arroba.http_request_with_credential` with this exact JSON argument:',
        JSON.stringify({
          credential_id: generatedCredentialId,
          method: 'GET',
          url: `http://127.0.0.1:${echo.port}/generated`,
          max_response_bytes: 4096,
        }),
        'When the HTTP response body shows verified true, reply exactly M17_GENERATED_CREDENTIAL_DONE.',
      ].join('\n'), []))

      const vaultUnlock = await respondToVaultUnlockInteraction({
        client,
        sessionId: session.id,
        agentId: agent.id,
        passphrase: vaultPassphrase,
        getSessionStateRequest,
        respondToInteractionRequest,
        timeoutMs: Math.min(options.timeoutMs, 240_000),
        pollMs: options.pollMs,
      })

      const generatedCreateCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: generatedStartedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('create_generated_credential') &&
          update.status === 'completed' &&
          update.input?.credential?.id === generatedCredentialId,
      })
      const generatedUseCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: generatedStartedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('http_request_with_credential') &&
          update.status === 'completed' &&
          update.input?.credential_id === generatedCredentialId,
      })
      const generatedEchoCall = await waitForVerifiedEchoCall({
        calls: echo.calls,
        route: '/generated',
        sinceIndex: generatedEchoStart,
        timeoutMs: 30_000,
        pollMs: options.pollMs,
      })

      const userStartedAt = Date.now()
      const userEchoStart = echo.calls.length
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is the user-entered credential half of the M17 live drill.',
        'Use Arroba runtime MCP tools only. Do not ask me in chat for the secret.',
        'Step 1: call `arroba.request_credential_secret` with this exact JSON argument:',
        JSON.stringify({
          credential: {
            id: userCredentialId,
            description: `User-entered credential drill for ${provider}`,
            allowed_hosts: [`127.0.0.1:${echo.port}`],
            allowed_uses: ['http'],
            injection: { kind: 'header', name: 'authorization', value: 'Bearer ${secret}' },
          },
          prompt: {
            title: 'M17 credential drill',
            message: 'Enter the drill password to store in Arroba Vault.',
            placeholder: 'Password',
            min_length: 8,
            max_length: 128,
            timeout_sec: 180,
          },
          overwrite: false,
        }),
        'After the tool returns status stored, call `arroba.http_request_with_credential` with this exact JSON argument:',
        JSON.stringify({
          credential_id: userCredentialId,
          method: 'GET',
          url: `http://127.0.0.1:${echo.port}/user`,
          max_response_bytes: 4096,
        }),
        'When the HTTP response body shows verified true, reply exactly M17_USER_CREDENTIAL_DONE.',
      ].join('\n'), []))

      const interaction = await respondToSecretInteraction({
        client,
        sessionId: session.id,
        agentId: agent.id,
        secret: userSecret,
        getSessionStateRequest,
        respondToInteractionRequest,
        timeoutMs: Math.min(options.timeoutMs, 240_000),
        pollMs: options.pollMs,
      })
      const userCreateCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: userStartedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('request_credential_secret') &&
          update.status === 'completed' &&
          update.input?.credential?.id === userCredentialId,
      })
      const userUseCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: userStartedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('http_request_with_credential') &&
          update.status === 'completed' &&
          update.input?.credential_id === userCredentialId,
      })
      const userEchoCall = await waitForVerifiedEchoCall({
        calls: echo.calls,
        route: '/user',
        sinceIndex: userEchoStart,
        timeoutMs: 30_000,
        pollMs: options.pollMs,
      })
      await waitForAgentPromptIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        getSessionStateRequest,
        timeoutMs: Math.min(options.timeoutMs, 30_000),
        pollMs: options.pollMs,
      }).catch(() => {})

      const transcript = await providerTranscript({ historyDir, agentId: agent.id, sinceMs: generatedStartedAt })
      assertNoSecretLeak(transcript, userSecret, provider)
      assertNoSecretLeak(transcript, vaultPassphrase, `${provider} vault passphrase`)
      await mkdir(artifactsDir, { recursive: true })
      await writeFile(
        path.join(artifactsDir, `m17-agent-vault-credential-${provider}-history.json`),
        JSON.stringify(transcript, null, 2),
        'utf8',
      )
      const screenshot = await renderTerminalScreenshot(`m17-agent-vault-credential-${provider}.png`, `M17 Agent Vault Credential Drill (${provider})`, [
        `PASS encrypted Arroba vault popup title="${vaultUnlock.title}" input_kind=${vaultUnlock.custom_choice?.input_kind ?? 'missing'}`,
        'PASS vault popup was answered with a fixed TTL choice plus kernel-only custom passphrase',
        'PASS provider agent called arroba.create_generated_credential',
        `PASS generated credential used through ${generatedUseCall.tool}`,
        `PASS verifier received generated credential request auth_length=${generatedEchoCall.authLength}`,
        'PASS provider agent called arroba.request_credential_secret',
        `PASS redacted interaction input_kind=${interaction.custom_choice?.input_kind ?? 'missing'}`,
        `PASS user-entered credential used through ${userUseCall.tool}`,
        `PASS verifier received user credential request auth_length=${userEchoCall.authLength}`,
        'PASS provider history artifact contains neither the user-entered secret nor the vault passphrase',
      ])

      results.push({
        provider,
        agentId: agent.id,
        generatedCredentialId,
        userCredentialId,
        generatedCreateTool: generatedCreateCall.tool,
        generatedUseTool: generatedUseCall.tool,
        generatedVerifiedAuthLength: generatedEchoCall.authLength,
        userCreateTool: userCreateCall.tool,
        userUseTool: userUseCall.tool,
        userVerifiedAuthLength: userEchoCall.authLength,
        vaultUnlockTitle: vaultUnlock.title,
        vaultUnlockChoiceIds: (vaultUnlock.choices ?? []).map((choice) => choice.id),
        screenshot,
      })
      await client.send(teardownProviderProcessesRequest(provider, true)).catch(() => {})
      const deadline = Date.now() + 60_000
      while (Date.now() < deadline) {
        const processes = unwrapVariant(await client.send(listProviderProcessesRequest(provider)), 'ProviderProcessesListed').processes ?? []
        if (!processes.some((process) => process.provider === provider)) break
        await sleep(options.pollMs)
      }
    }

    for (const key of credentialKeys) {
      await client.send(deleteCredentialSecretRequest(key)).catch(() => {})
    }
    await writeFile(
      path.join(artifactsDir, 'm17-agent-vault-credential-live-drill.json'),
      JSON.stringify({ ok: true, kernelUrl, workspace, providers: results }, null, 2),
      'utf8',
    )
    console.log(JSON.stringify({ ok: true, kernelUrl, workspace, providers: results }, null, 2))
    succeeded = true
    await client.send(endSessionRequest(session.id)).catch(() => {})
    sessionId = null
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) {
      if (deleteCredentialSecretRequest) {
        for (const key of credentialKeys) {
          await client.send(deleteCredentialSecretRequest(key)).catch(() => {})
        }
      }
      if (sessionId && endSessionRequest) await client.send(endSessionRequest(sessionId)).catch(() => {})
      await client.close().catch(() => {})
    }
    await echo?.close?.().catch(() => {})
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'agent-vault-credential',
        providers: options.providers.join(','),
        model: options.model,
        providerModels: options.providerModels,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
        historyDir,
        capabilityRoot,
        credentialKeyCount: credentialKeys.length,
        results,
      },
      log: (name, details) => console.log(`[agent-vault-credential-drill] ${name}`, JSON.stringify(details)),
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

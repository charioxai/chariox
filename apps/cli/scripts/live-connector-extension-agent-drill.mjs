#!/usr/bin/env node
import http from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_PROVIDERS = ['codex', 'opencode', 'claude']
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000
const realHomeDir = process.env.HOME || homedir()

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

function parseArgs(argv) {
  const options = { providers: DEFAULT_PROVIDERS, providerModels: {}, model: DEFAULT_MODEL, effort: 'low', timeoutMs: DEFAULT_TIMEOUT_MS, pollMs: DEFAULT_POLL_MS, keepArtifactsOnFailure: false }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown option: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-connector-extension-agent-drill.mjs [options]',
    '',
    'Runs a provider-backed connector extension drill:',
    '- registers a vault-backed credential and HTTP connector',
    '- grants the connector to real provider agents',
    '- prompts each agent to call connector tools with fixed inputs',
    '- verifies a local API received the vault secret while the model only sees API results',
    '- verifies each agent writes observed connector outputs through Workspace Live Sync',
  ].join('\n'))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const log = (message, details) => console.log(details === undefined ? `[connector-agent-drill] ${message}` : `[connector-agent-drill] ${message} ${JSON.stringify(details)}`)

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: options.cwd ?? repoRoot, env: options.env ?? process.env, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel', '--bin', 'arroba-adapter-http'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function makePorts() {
  const base = 62000 + Math.floor(Math.random() * 1000)
  return { kernelPort: base, mcpPort: base + 1000, opencodePort: base + 2000, codexPort: base + 2001 }
}

function startDaemon(binary, env) {
  const child = spawn(binary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdoutText = ''
  child.stderrText = ''
  child.stdout.on('data', (chunk) => { child.stdoutText += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.stderrText += chunk.toString() })
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
}

async function waitForDaemon(kernelUrl) {
  const deadline = Date.now() + 30_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(requests.listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

function unwrap(response, key) {
  if (!response?.[key]) throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  return response[key]
}

function unwrapOne(response, ...keys) {
  for (const key of keys) if (response?.[key]) return response[key]
  throw new Error(`expected one of ${keys.join(', ')}, got ${JSON.stringify(response)}`)
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && options.model === DEFAULT_MODEL) return 'opencode/gpt-5.4'
  if (provider === 'opencode' && !options.model.includes('/')) return `openai/${options.model}`
  return options.model
}

function startApiServer(expectedSecret) {
  const seen = []
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1')
    seen.push({ pathname: url.pathname, secret: url.searchParams.get('key'), query: url.searchParams.get('q') })
    res.setHeader('content-type', 'application/json')
    if (url.pathname === '/public') return res.end(JSON.stringify({ route: 'public', echo: url.searchParams.get('q') }))
    if (url.pathname === '/secret') return res.end(JSON.stringify({ route: 'secret', authorized: url.searchParams.get('key') === expectedSecret, code: 'vault-ok' }))
    res.statusCode = 404
    res.end(JSON.stringify({ error: 'not_found' }))
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port, seen }))
  })
}

async function registerConnector(client, root, port, vaultKey) {
  const adapterDir = path.join(root, 'http-adapter')
  const adapterPath = path.join(adapterDir, 'adapter.yaml')
  const credentialPath = path.join(root, 'agent-credential.yaml')
  const connectorPath = path.join(root, 'agent-connector.yaml')
  await mkdir(adapterDir, { recursive: true })
  await writeFile(adapterPath, `
kind: connector_adapter
name: http
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${path.join(repoRoot, 'apps/kernel/target/debug/arroba-adapter-http')}
description: HTTP adapter agent drill build.
`, 'utf8')
  await writeFile(credentialPath, `
id: agent-local-api
description: Agent connector drill key
source:
  type: vault
  key: ${vaultKey}
allowed_hosts:
  - 127.0.0.1:${port}
allowed_uses:
  - connector
injection:
  kind: query
  name: key
`, 'utf8')
  await writeFile(connectorPath, `
kind: connector
name: agent_local_api
description: Local HTTP API connector for provider drills.
adapter: http
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: public_echo
    description: Read public echo data.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    config:
      base_url: http://127.0.0.1:${port}
      method: GET
      path: /public
      query:
        q: "{{q}}"
  - name: secret_status
    description: Read vault-backed authorization status.
    safety: read
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    config:
      base_url: http://127.0.0.1:${port}
      method: GET
      path: /secret
`, 'utf8')
  await client.send(requests.registerConnectorAdapterRequest(adapterPath))
  await client.send(requests.registerCredentialRequest(credentialPath))
  await client.send(requests.registerConnectorRequest(connectorPath))
}

async function waitForProviderRunReady(client, providerRunId, timeoutMs) {
  const deadline = Date.now() + Math.min(timeoutMs, 90_000)
  let lastRun = null
  while (Date.now() < deadline) {
    lastRun = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (lastRun?.state === 'Running') return
    if (lastRun?.state === 'Ended') throw new Error(`provider run ended before ready: ${JSON.stringify(lastRun)}`)
    await sleep(500)
  }
  throw new Error(`timed out waiting for provider run ready\n${JSON.stringify(lastRun)}`)
}

async function waitForPromptDone(client, sessionId, attachmentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const response = await client.send(requests.getSessionStateRequest(sessionId))
    lastState = response.SessionState?.session ?? null
    if (!lastState?.active_prompt) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for prompt completion\n${JSON.stringify(lastState)}`)
}

async function waitForFile(file, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      return await readFile(file, 'utf8')
    } catch {
      await sleep(pollMs)
    }
  }
  throw new Error(`timed out waiting for ${file}`)
}

async function runProviderScenario({ client, session, attachment, workspace, outputsDir, provider, options }) {
  const model = modelForProvider(provider, options)
  const agent = unwrap(await client.send(requests.spawnAgentRequest(session.id, provider, `${provider}-connector-extension`, model, workspace, options.effort)), 'AgentSpawned').agent
  await client.send(requests.grantAgentExtensionRequest(workspace, agent.id, 'connector', 'agent_local_api', null, { credential: 'agent-local-api', maxSafety: 'read' }))
  const launch = unwrapOne(await client.send(requests.launchProviderRunRequest(session.id, provider, 'default', model, options.effort, agent.id)), 'ProviderRunLaunched', 'ProviderRunLaunchAccepted').provider_run
  await waitForProviderRunReady(client, launch.id, options.timeoutMs)
  const outputRel = `outputs/${provider}-connector-extension-result.json`
  const outputPath = path.join(outputsDir, `${provider}-connector-extension-result.json`)
  const prompt = [
    'This is an Arroba connector extension end-to-end drill.',
    'You have exactly two relevant connector tools: `agent_local_api_public_echo` and `agent_local_api_secret_status`.',
    'Call `agent_local_api_public_echo` with {"q":"connector-alpha"}.',
    'Call `agent_local_api_secret_status` with {}.',
    `Then write ${outputRel} using Arroba workspace live sync as one JSON object with these keys:`,
    'public_route, public_echo, secret_route, secret_authorized, secret_code.',
    'Use only values returned by the connector tools. Do not use placeholders. Reply exactly CONNECTOR_EXTENSION_AGENT_DRILL_DONE.',
  ].join('\n')
  await client.send(requests.submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
  await waitForPromptDone(client, session.id, attachment.id, options.timeoutMs, options.pollMs)
  const parsed = JSON.parse(await waitForFile(outputPath, options.timeoutMs, options.pollMs))
  const expected = { public_route: 'public', public_echo: 'connector-alpha', secret_route: 'secret', secret_authorized: true, secret_code: 'vault-ok' }
  const mismatches = Object.entries(expected).filter(([key, value]) => parsed?.[key] !== value)
  if (mismatches.length > 0) throw new Error(`${provider} wrote unexpected result\nexpected=${JSON.stringify(expected)}\nactual=${JSON.stringify(parsed)}`)
  log('provider-pass', { provider, agentId: agent.id, output: outputRel })
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) return printHelp()
  const root = path.join(repoRoot, 'target', 'live-connector-extension-agent-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(root, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const arrobaHome = path.join(root, 'arroba-home')
  const vaultKey = `connector-agent-drill-${process.pid}-${Date.now()}`
  const secretValue = `connector-agent-secret-${process.pid}-${Date.now()}`
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let daemon = null
  let client = null
  let api = null
  let succeeded = false
  try {
    await mkdir(outputsDir, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(path.join(workspace, 'README.md'), '# connector extension agent drill\n', 'utf8')
    api = await startApiServer(secretValue)
    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, {
      ...process.env,
      HOME: realHomeDir,
      CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, '.codex'),
      OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, '.config', 'opencode'),
      XDG_DATA_HOME: process.env.XDG_DATA_HOME ?? path.join(realHomeDir, '.local', 'share'),
      XDG_CACHE_HOME: process.env.XDG_CACHE_HOME ?? path.join(realHomeDir, '.cache'),
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
      ARROBA_HOME: arrobaHome,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: `connector-agent-drill-${process.pid}-${Date.now()}`,
      ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    })
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    await client.send(requests.setCredentialSecretRequest(vaultKey, secretValue))
    await registerConnector(client, root, api.port, vaultKey)
    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, 'connector-extension-agent-drill')), 'SessionCreated').session
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(session.id, `connector-agent-drill-${process.pid}`)), 'SessionAttached').attachment
    for (const provider of options.providers) await runProviderScenario({ client, session, attachment, workspace, outputsDir, provider, options })
    if (!api.seen.some((entry) => entry.pathname === '/secret' && entry.secret === secretValue)) throw new Error('API server did not receive vault-injected secret')
    await client.send(requests.deleteCredentialSecretRequest(vaultKey)).catch(() => {})
    succeeded = true
    log('pass', { providers: options.providers, workspace })
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    if (api) await new Promise((resolve) => api.server.close(resolve))
    if (succeeded && !options.keepArtifactsOnFailure) await rm(root, { recursive: true, force: true })
    else log('artifacts-kept', { root, daemonStdout: daemon?.stdoutText?.slice(-2000), daemonStderr: daemon?.stderrText?.slice(-4000) })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

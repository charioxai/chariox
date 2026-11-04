#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 VAUDIT S2 live drill: real provider agents,
// under an owner-approved sudo window, capture an API key a service CLI
// prints (chariox.vault.capture_command) and then call the service with it
// injected as an environment variable (chariox.vault.use_env). The service
// must accept the key while no transcript, history, state, log or provider
// file ever holds it. --expect-missing is the red run against a kernel
// without the tools: it passes when the agent cannot capture or use a key.
import http from 'node:http'
import os from 'node:os'
import { randomBytes } from 'node:crypto'
import { spawnOwned, signalOwnedProcess } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'
import { mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { createWriteStream } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..', '..')
const MODELS = { codex: 'gpt-6-luna', claude: 'claude-haiku-5-5' }
// Tool-call records (Claude names MCP tools mcp__chariox__chariox_*), not mentions in the prompt.
const CAPTURE = /tool\\*"\s*:\s*\\*"(?:mcp__chariox__chariox_vault_capture_command|chariox\.vault\.capture_command)/
const USE_ENV = /tool\\*"\s*:\s*\\*"(?:mcp__chariox__chariox_vault_use_env|chariox\.vault\.use_env)/
const WRITE_FILE = /tool\\*"\s*:\s*\\*"(?:mcp__chariox__chariox_vault_write_file|chariox\.vault\.write_file)/

function usage() {
  return `usage: CHARIOX_KERNEL_BINARY=<kernel> live-vault-command-drill.mjs --root <dir> --evidence <dir>
  [--providers codex,claude] [--codex-home <dir>] [--claude-token-file <file>]
  [--expect-missing] [--lifecycle] [--timeout-ms 300000]`
}

function parseArgs(argv) {
  const options = { providers: ['codex', 'claude'], timeoutMs: 300_000, expectMissing: false }
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    const value = () => {
      const next = argv[++index]
      if (!next) throw new Error(`${flag} needs a value\n${usage()}`)
      return next
    }
    if (flag === '--providers') options.providers = value().split(',').filter(Boolean)
    else if (flag === '--root') options.root = path.resolve(value())
    else if (flag === '--evidence') options.evidence = path.resolve(value())
    else if (flag === '--codex-home') options.codexHome = path.resolve(value())
    else if (flag === '--claude-token-file') options.claudeTokenFile = path.resolve(value())
    else if (flag === '--timeout-ms') options.timeoutMs = Number(value())
    else if (flag === '--expect-missing') options.expectMissing = true
    else if (flag === '--lifecycle') options.lifecycle = true
    else throw new Error(`unknown argument ${flag}\n${usage()}`)
  }
  if (!process.env.CHARIOX_KERNEL_BINARY || !options.root || !options.evidence) throw new Error(usage())
  if (options.providers.some((p) => !MODELS[p])) throw new Error(`providers are codex and claude\n${usage()}`)
  if (options.providers.includes('codex') && !options.codexHome) throw new Error('--codex-home is required for codex')
  if (options.providers.includes('claude') && !options.claudeTokenFile) throw new Error('--claude-token-file is required for claude')
  return options
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const step = (label) => console.error(`${new Date().toISOString()} ${label}`)

function unwrap(response, ...variants) {
  for (const variant of variants) if (response?.[variant] !== undefined) return response[variant]
  throw new Error(`expected ${variants.join(' or ')}, got ${Object.keys(response ?? {}).join(',') || 'nothing'}`)
}

// The fake service: POST /keys mints a key shown once; GET /me needs it.
function startService() {
  const keys = new Set()
  const calls = []
  const server = http.createServer((request, response) => {
    const send = (status, body) => {
      response.writeHead(status, { 'content-type': 'application/json' })
      response.end(JSON.stringify(body))
    }
    if (request.method === 'POST' && request.url === '/keys') {
      const secret = `svc_live_${randomBytes(18).toString('hex')}`
      keys.add(secret)
      calls.push({ path: '/keys', at: Date.now() })
      return send(201, { key_id: `key_${keys.size}`, secret })
    }
    if (request.method === 'GET' && request.url === '/me') {
      const token = String(request.headers.authorization ?? '').replace(/^Bearer /, '')
      const authorized = keys.has(token)
      calls.push({ path: '/me', authorized, at: Date.now() })
      return authorized ? send(200, { user: 'drill-owner' }) : send(401, { error: 'unauthorized' })
    }
    send(404, { error: 'not found' })
  })
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () =>
    resolve({ server, keys, calls, port: server.address().port })))
}

// The service CLI the agent runs through the kernel. It echoes the key on
// purpose, so masking of the command's own output is exercised.
function serviceCli(port) {
  return `const base = 'http://127.0.0.1:${port}'
const command = process.argv[2]
if (command === 'create-key') {
  const response = await fetch(base + '/keys', { method: 'POST' })
  console.log(JSON.stringify(await response.json()))
} else if (command === 'whoami') {
  const key = process.env.SVC_API_KEY
  if (!key) { console.error('SVC_API_KEY is not set'); process.exit(2) }
  const response = await fetch(base + '/me', { headers: { authorization: 'Bearer ' + key } })
  if (!response.ok) { console.error('unauthorized (' + response.status + ') for key ' + key); process.exit(1) }
  console.log('authenticated as ' + (await response.json()).user + ' using key ' + key)
} else { console.error('usage: svc.mjs create-key|whoami'); process.exit(64) }
`
}

async function sessionState(client, requests, sessionId) {
  const state = unwrap(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
  return state.session ?? state
}

async function waitFor(label, timeoutMs, probe) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const value = await probe()
    if (value) return value
    await sleep(1000)
  }
  throw new Error(`timed out waiting for ${label}`)
}

async function idle(client, requests, sessionId, attachmentId, agentId, timeoutMs) {
  await waitFor(`agent ${agentId} to finish`, timeoutMs, async () => {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const session = await sessionState(client, requests, sessionId)
    const prompts = session.prompt_states?.[agentId] ?? {}
    const agent = (session.agents ?? []).find((candidate) => candidate.id === agentId)
    return !prompts.active_prompt && !(prompts.queued_prompts ?? []).length && agent && !agent.is_processing && agent.state !== 'Working'
  })
}

// The agent's transcript as clients read it: the history outline with its
// prefetched content and every referenced blob.
async function transcript(client, requests, sessionId, agentId) {
  const first = (response) => Object.values(response ?? {})[0]
  const outline = first(await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], null, null, true)))
  const text = JSON.stringify(outline)
  const blobs = [...new Set([...text.matchAll(/"blob_id":"([^"]+)"/g)].map((match) => match[1]))]
  const contents = blobs.length
    ? first(await client.send(requests.getSessionHistoryBlobContentsRequest(sessionId, agentId, blobs)))
    : null
  return `${text}\n${JSON.stringify(contents)}`
}

// Boolean only: never print or copy what is scanned.
async function filesContaining(roots, needles, sinceMs) {
  const hits = []
  const visit = async (dir) => {
    for (const entry of await readdir(dir, { withFileTypes: true }).catch(() => [])) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) { await visit(full); continue }
      if (!entry.isFile() || /auth\.json$|credential|token/i.test(entry.name)) continue
      const info = await stat(full).catch(() => null)
      if (!info || info.size > 64 * 1024 * 1024 || (sinceMs && info.mtimeMs < sinceMs)) continue
      const text = await readFile(full, 'latin1').catch(() => '')
      if (needles.some((needle) => text.includes(needle))) hits.push(full)
    }
  }
  for (const root of roots) await visit(root)
  return hits
}

async function linkProvider(client, requests, provider, options) {
  if (provider === 'codex') {
    return unwrap(await client.send(requests.linkProviderAccountProfileRequest('codex', 'S2 drill Codex', options.codexHome)),
      'ProviderAccountProfile').profile.profile_id
  }
  const profile = unwrap(await client.send(requests.createProviderAccountProfileRequest('claude', 'S2 drill Claude')),
    'ProviderAccountProfile').profile
  const token = (await readFile(options.claudeTokenFile, 'utf8')).trim()
  unwrap(await client.send(requests.setProviderAccountCredentialRequest('claude', profile.profile_id, token, false)),
    'ProviderAccountCredentialStored')
  return profile.profile_id
}

function prompt(provider, service, node, workspace) {
  return [
    'Set up access to the local Svc service for this project without ever seeing its API key.',
    `1. Call the chariox.vault.capture_command tool with request_id "svc-key-${provider}", argv ["${node}", "svc.mjs", "create-key"],`,
    `host "127.0.0.1:${service.port}", label "Svc API key", use_binding {"target":{"kind":"command_environment","name":"SVC_API_KEY","origin":"http://127.0.0.1:${service.port}"}} and extract {"kind": "json", "pointer": "/secret"}. It returns a credential_id.`,
    `2. Call the chariox.vault.use_env tool with argv ["${node}", "svc.mjs", "whoami"] with origin "http://127.0.0.1:${service.port}" and env {"SVC_API_KEY": "<that credential_id>"}.`,
    `3. Capture a second key with capture_command request_id "svc-file-${provider}", the same argv/host/extract and use_binding {"target":{"kind":"file","path":${JSON.stringify(path.join(workspace, `.env.${provider}`))}}}.`,
    `4. Call the chariox.vault.write_file tool with path ".env.${provider}", template "SVC_API_KEY={{KEY}}\\n" and values {"KEY": "<the second credential_id>"}.`,
    '5. Reply with the whoami stdout the tool returned, exactly. Do not run svc.mjs any other way, do not read files, and never try to see the key.',
  ].join('\n')
}

// MP-08 / MP-09 / MP-10 / MP-11 F1/F2: real provider MCP calls on a private kernel.
async function commandLifecycle({ client, requests, session, attachment, agentId, provider, workspace, url, env, options }) {
  const runs = path.join(workspace, `capture-runs-${provider}`)
  const started = path.join(workspace, `use-started-${provider}`)
  const marker = path.join(workspace, `must-not-write-${provider}`)
  const argv = [process.execPath, '-e', `require('fs').appendFileSync(${JSON.stringify(runs)}, 'run\\n'); setTimeout(() => console.log(require('crypto').randomBytes(24).toString('hex')), 8000)`]
  const origin = 'https://api.example.com'
  const args = { request_id: 'overlapping-live-capture', argv, host: 'api.example.com',
    use_binding: { target: { kind: 'command_environment', name: 'KEY', origin } } }
  const submit = async (text) => {
    const connection = new (await import('../../../packages/kernel-client/dist/ipc.js')).LocalIpcClient(url, { localAuthEnvironment: env })
    try { await connection.send(requests.submitPromptRequest(session.id, attachment.id, agentId, text, [])) }
    finally { await connection.close().catch(() => {}) }
  }
  step(`${provider} F2 overlapping capture`)
  await submit([
    'Run exactly two concurrent calls of chariox.vault.capture_command with the identical arguments below.',
    'Issue both in the same parallel tool batch; do not wait for the first before issuing the second. The first sleeps for 8 seconds.',
    JSON.stringify(args),
    'An already-running error for one call is expected. Then retry the same request once after the successful result.',
    'Do not run other tools, read files, or try to see the secret. Reply only with the credential_id.',
  ].join('\n'))
  await idle(client, requests, session.id, attachment.id, agentId, options.timeoutMs)
  const seen = await transcript(client, requests, session.id, agentId)
  const credentials = unwrap(await client.send(requests.listCredentialsRequest()), 'CredentialsListed').credentials ?? []
  const captured = credentials.filter(c => c.metadata?.created_by_id === agentId && c.metadata?.created_by_kind === 'vault_capture')
  const count = (await readFile(runs, 'utf8').catch(() => '')).trim().split('\n').filter(Boolean).length
  const row = { provider, model: MODELS[provider], capture_runs: count, captured_credentials: captured.length,
    overlap_refused: seen.includes('already running'), capture_seen: CAPTURE.test(seen) }
  await writeFile(path.join(options.evidence, `lifecycle-capture-${provider}.json`), seen)
  if (count !== 1 || captured.length !== 1 || !row.overlap_refused || !row.capture_seen)
    throw Error(`F2 did not prove overlapping admission: ${JSON.stringify(row)}`)
  step(`${provider} F1 cancel delayed child write`)
  const use = { argv: ['/bin/sh', '-c', `touch '${started}'; (sleep 8; touch '${marker}') & wait`],
    origin, env: { KEY: captured[0].id } }
  await submit(`Call chariox.vault.use_env exactly once with ${JSON.stringify(use)}. Do not run other tools. Wait for its result.`)
  await waitFor('the delayed Vault command to launch', options.timeoutMs, async () => stat(started).then(() => true).catch(() => false))
  await client.send(requests.cancelActivePromptRequest(session.id, attachment.id, agentId))
  await idle(client, requests, session.id, attachment.id, agentId, options.timeoutMs)
  await sleep(9000)
  row.cancelled_write_absent = !(await stat(marker).then(() => true).catch(() => false))
  const after = await transcript(client, requests, session.id, agentId)
  await writeFile(path.join(options.evidence, `lifecycle-cancel-${provider}.json`), after)
  row.use_env_seen = USE_ENV.test(after)
  row.pass = row.cancelled_write_absent && row.use_env_seen
  if (!row.pass) throw Error(`F1 delayed write survived cancellation: ${JSON.stringify(row)}`)
  return row
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const runId = `${Date.now()}`
  const rootDir = path.join(options.root, runId)
  const workspace = path.join(rootDir, 'workspace')
  const charioxHome = path.join(rootDir, 'chariox-home')
  const vaultPath = path.join(rootDir, 'vault', 'vault.json')
  const passphrase = randomBytes(24).toString('base64url')
  const base = 40000 + Math.floor(Math.random() * 20000)
  const env = {
    ...process.env,
    CHARIOX_HOME: charioxHome,
    // A private HOME: no default path can reach a shared Vault or login.
    HOME: path.join(rootDir, 'home'),
    XDG_STATE_HOME: path.join(rootDir, 'xdg-state'),
    CHARIOX_KERNEL_PORT: String(base),
    CHARIOX_MCP_PORT: String(base + 1),
    CHARIOX_OPENCODE_PORT: String(base + 2),
    CHARIOX_CODEX_PORT: String(base + 3),
    CHARIOX_DAEMON_ID: `s2-vault-command-${runId}`,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'k.sock'),
    CHARIOX_CAPABILITY_ISOLATION_ROOT: path.join(rootDir, 'capabilities'),
  }
  await mkdir(workspace, { recursive: true })
  await mkdir(charioxHome, { recursive: true })
  await mkdir(path.join(rootDir, 'home'), { recursive: true })
  await mkdir(options.evidence, { recursive: true })
  // With CHARIOX_HOME set the kernel reads its config there; the Vault must
  // never fall back to the default path under HOME.
  await writeFile(path.join(charioxHome, 'config.toml'), [
    'version = 1', '', '[credential_vault]', `service = "chariox-s2-drill-${runId}"`,
    `path = "${vaultPath}"`, 'backend = "chariox_encrypted"',
    'unlock_policy = "ttl"', 'default_ttl_minutes = 30', 'max_ttl_minutes = 240', 'agent_management = "allow"', '',
  ].join('\n'))
  const service = await startService()
  await writeFile(path.join(workspace, 'svc.mjs'), serviceCli(service.port))
  const log = createWriteStream(path.join(rootDir, 'kernel.log'))
  const kernel = spawnOwned(process.env.CHARIOX_KERNEL_BINARY, [], { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] })
  kernel.stdout.pipe(log)
  kernel.stderr.pipe(log)
  const report = { mode: options.expectMissing ? 'expect-missing' : 'pass', kernelPid: kernel.pid, providers: [] }
  const [{ LocalIpcClient }, requests] = await Promise.all([
    import('../../../packages/kernel-client/dist/ipc.js'),
    import('../../../packages/kernel-client/dist/ipc-requests.js'),
  ])
  let client = null
  try {
    const url = `ws://127.0.0.1:${base}`
    client = await waitFor('the kernel', 60_000, async () => {
      const probe = new LocalIpcClient(url, { localAuthEnvironment: env })
      try {
        await probe.send(requests.getCredentialVaultStatusRequest())
        return probe
      } catch {
        await probe.close().catch(() => {})
        return null
      }
    })
    step('kernel ready')
    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, `s2-${runId}`,
      undefined, undefined, 'unrestricted')), 'SessionCreated').session
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(session.id, `s2-${runId}`)), 'SessionAttached').attachment
    // The owner opens the Vault menu; a new kernel's first-time prompt creates
    // the Vault, whose passphrase is also the sudo passkey.
    // The request waits for the prompt's answer, so it runs on its own connection.
    const menu = new LocalIpcClient(url, { localAuthEnvironment: env })
    const opened = menu.send(requests.manageCredentialVaultRequest(session.id)).finally(() => menu.close().catch(() => {}))
    opened.catch(() => {})
    const create = await waitFor('the Vault creation prompt', 60_000, async () => (await sessionState(client, requests, session.id))
      .active_interactions?.find((entry) => entry.custom_choice?.input_kind === 'secret'))
    step(`Vault prompt: ${create.title}`)
    await client.send(requests.respondToInteractionRequest(session.id, create.id, create.custom_choice.id, passphrase))
    await Promise.race([opened, sleep(60_000).then(() => { throw new Error('the Vault menu request did not finish') })])
    await stat(vaultPath).catch(() => { throw new Error('the kernel did not create the Vault at the configured path') })
    step('Vault created')
    await waitFor('the Vault to open', 60_000, async () =>
      unwrap(await client.send(requests.getCredentialVaultStatusRequest()), 'CredentialVaultStatus')?.status?.unlocked)
    for (const provider of options.providers) {
      const row = { provider, model: MODELS[provider] }
      report.providers.push(row)
      const profile = await linkProvider(client, requests, provider, options)
      // The owner's account check verifies the linked login before a run may use it.
      const auth = Object.values(await client.send(requests.getProviderAuthStatusRequest(provider, profile)))[0]
      row.auth_state = auth?.status?.auth_state ?? auth?.auth_state ?? null
      step(`${provider} profile linked (${row.auth_state})`)
      const agent = unwrap(await client.send(requests.spawnAgentRequest(session.id, provider, `s2-${provider}`, MODELS[provider],
        workspace, 'low', 'build', undefined, undefined, undefined, undefined, profile)), 'AgentSpawned')
      const agentId = agent.agent?.id ?? agent.agent_id
      const run = unwrap(await client.send(requests.launchProviderRunRequest(session.id, provider, profile, MODELS[provider], 'low', agentId)),
        'ProviderRunLaunchAccepted', 'ProviderRunLaunched').provider_run
      await waitFor(`${provider} run`, 120_000, async () =>
        (await sessionState(client, requests, session.id)).active_provider_run_id === run.id)
      step(`${provider} run active`)
      row.resolved_model = run.model ?? null
      if (options.lifecycle) {
        Object.assign(row, await commandLifecycle({ client, requests, session, attachment, agentId, provider, workspace, url, env, options }))
        continue
      }
      const since = Date.now()
      const keysBefore = new Set(service.keys)
      const submitter = new LocalIpcClient(url, { localAuthEnvironment: env })
      await submitter.send(requests.submitPromptRequest(session.id, attachment.id, agentId,
        prompt(provider, service, process.execPath, workspace), [])).finally(() => submitter.close().catch(() => {}))
      await idle(client, requests, session.id, attachment.id, agentId, options.timeoutMs)
      step(`${provider} turn finished`)
      const minted = [...service.keys].filter((key) => !keysBefore.has(key))
      const seen = await transcript(client, requests, session.id, agentId)
      await writeFile(path.join(options.evidence, `transcript-${report.mode}-${provider}.json`), seen)
      const credentials = unwrap(await client.send(requests.listCredentialsRequest()), 'CredentialsListed').credentials ?? []
      const captured = credentials.filter((c) => c.metadata?.created_by_id === agentId && c.metadata?.created_by_kind === 'vault_capture')
      const state = JSON.stringify(await sessionState(client, requests, session.id))
      // The .env the agent asked the kernel to write holds the key by design.
      const envFile = path.join(workspace, `.env.${provider}`)
      const envText = await readFile(envFile, 'utf8').catch(() => null)
      const leaks = (minted.length ? await filesContaining([rootDir], minted) : []).filter((file) => file !== envFile)
      const providerLeaks = provider === 'codex' && minted.length
        ? await filesContaining([path.join(options.codexHome, 'sessions')], minted, since) : []
      Object.assign(row, {
        capture_tool_in_transcript: CAPTURE.test(seen),
        use_env_tool_in_transcript: USE_ENV.test(seen),
        write_file_tool_in_transcript: WRITE_FILE.test(seen),
        env_file_holds_key: minted.length === 2 && envText?.replace(/\\n$/, '').trim() === `SVC_API_KEY=${minted[1]}`,
        keys_minted: minted.length,
        captured_credentials: captured.length,
        captured_injection_env: captured.some((c) => c.injection?.kind === 'env') && captured.some((c) => c.injection?.kind === 'file'),
        service_authorized_calls: service.calls.filter((c) => c.path === '/me' && c.authorized && c.at >= since).length,
        masked_output_in_transcript: seen.includes('[vault:cap-'),
        key_in_transcript: minted.some((key) => seen.includes(key)),
        key_in_session_state: minted.some((key) => state.includes(key)),
        key_in_kernel_files: leaks.map((file) => path.relative(rootDir, file)),
        key_in_codex_sessions: providerLeaks.length,
      })
      row.pass = options.expectMissing
        ? !row.capture_tool_in_transcript && !row.use_env_tool_in_transcript && !row.write_file_tool_in_transcript &&
          row.captured_credentials === 0 && row.service_authorized_calls === 0 && envText === null
        : row.capture_tool_in_transcript && row.use_env_tool_in_transcript && row.keys_minted >= 1 &&
          row.captured_credentials === 2 && row.captured_injection_env && row.service_authorized_calls >= 1 &&
          row.masked_output_in_transcript && row.write_file_tool_in_transcript && row.env_file_holds_key &&
          !row.key_in_transcript && !row.key_in_session_state &&
          row.key_in_kernel_files.length === 0 && row.key_in_codex_sessions === 0
      console.log(JSON.stringify(row))
    }
    report.ok = report.providers.every((row) => row.pass)
  } catch (error) {
    report.ok = false
    report.error = String(error?.message ?? error).slice(0, 500)
  } finally {
    await client?.close().catch(() => {})
    if (kernel.exitCode == null) {
      signalOwnedProcess(kernel, 'SIGTERM')
      await Promise.race([new Promise((resolve) => kernel.once('exit', resolve)), sleep(8_000)])
      if (kernel.exitCode == null) signalOwnedProcess(kernel, 'SIGKILL')
    }
    service.server.close()
    await writeFile(path.join(options.evidence, `s2-vault-command-${report.mode}.json`), `${JSON.stringify(report, null, 2)}\n`)
    await rm(rootDir, { recursive: true, force: true })
  }
  console.log(JSON.stringify({ ok: report.ok, mode: report.mode, error: report.error }))
  process.exitCode = report.ok ? 0 : 1
}

await main()

import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { createReadStream } from 'node:fs'
import { once } from 'node:events'
import fs from 'node:fs/promises'
import http from 'node:http'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { createHash } from 'node:crypto'
import { setTimeout as sleep } from 'node:timers/promises'
import { terminalProviderOutputSnapshot } from './lib/remote-machine-runtime-output.mjs'

const execute = promisify(execFile)

// Real kernel and official OpenCode. Only the model endpoint is a fault
// fixture: it cannot produce a model reply or authorize tool execution.
const kernelBinary = process.env.CHARIOX_RETRY_KERNEL_BINARY
const opencodeBinary = process.env.CHARIOX_OPENCODE_BIN
const clientDist = process.env.CHARIOX_KERNEL_CLIENT_DIST
assert.ok([kernelBinary, opencodeBinary, clientDist].every(value => value && path.isAbsolute(value)),
  'set explicit absolute kernel, official OpenCode and built kernel-client paths')
const { LocalIpcClient } = await import(pathToFileURL(path.join(clientDist, 'ipc.js')))
const requests = await import(pathToFileURL(path.join(clientDist, 'ipc-requests.js')))
const scratch = path.join(os.homedir(), '.chariox/dev/browser-computer-use')
const workerThreads = process.env.CHARIOX_RETRY_WORKER_THREADS ?? '1'
assert.match(workerThreads, /^[1-4]$/, 'use one to four worker threads')
await fs.mkdir(scratch, { recursive: true })
const root = await fs.mkdtemp(path.join(scratch, 'opencode-kernel-retry.'))
const evidence = path.join(os.homedir(), '.codex/evidence/browser-computer-use', `opencode-kernel-retry-${Date.now()}.json`)
const report = { schema: 'chariox.opencode_kernel_retry.v1', status: 'running', modelRequests: 0, retries: [] }
report.pure = process.env.CHARIOX_RETRY_PURE === '1'
report.workerThreads = workerThreads
let kernel, client, session, failure, startupOutput = '', stage = 'setup'
const ports = []
function alive(pid) {
  try { process.kill(pid, 0); return true } catch (error) { if (error.code === 'ESRCH') return false; throw error }
}
async function listening(port) {
  return new Promise(resolve => {
    const socket = net.createConnection({ host: '127.0.0.1', port })
    const finish = value => { socket.destroy(); resolve(value) }
    socket.setTimeout(300, () => finish(false))
    socket.once('connect', () => finish(true))
    socket.once('error', () => finish(false))
  })
}
async function descendants(pid) {
  const { stdout } = await execute('ps', ['-axo', 'pid=,ppid='], { maxBuffer: 1024 * 1024 })
  const rows = stdout.trim().split('\n').map(line => line.trim().split(/\s+/).map(Number))
  const owned = new Set([pid])
  for (let previous = 0; previous !== owned.size;) {
    previous = owned.size
    for (const [child, parent] of rows) if (owned.has(parent)) owned.add(child)
  }
  return [...owned].filter(child => child !== pid)
}
const endpoint = http.createServer((request, response) => {
  report.modelRequests += 1
  let body = '', oversized = false
  request.on('data', chunk => {
    if (body.length + chunk.length > 262144) oversized = true
    else if (!oversized) body += chunk.toString()
  })
  request.on('end', () => {
    let titleRequest = false
    if (!oversized) {
      try {
        const payload = JSON.parse(body)
        const text = JSON.stringify(payload.instructions ?? '') + JSON.stringify((payload.messages ?? payload.input ?? []).filter(item => ['system', 'developer'].includes(item.role)))
        titleRequest = /you are a title generator/i.test(text)
      } catch {}
    }
    report.requestKinds ??= []
    report.requestKinds.push({ titleRequest, oversized, afterCancel: stage === 'cancel-retry' })
    response.writeHead(429, { 'content-type': 'application/json', 'retry-after': '2' })
    response.end(JSON.stringify({ error: { message: 'Chariox retry drill temporarily throttled', type: 'rate_limit_error', code: 'rate_limit_exceeded' } }))
  })
})
async function reservePort() {
  const server = net.createServer()
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  ports.push(port)
  return port
}
async function waitFor(probe, ms) {
  const deadline = Date.now() + ms
  do {
    const value = await probe()
    if (value) return value
    assert.ok(kernel?.exitCode === null && kernel?.signalCode === null, 'kernel exited')
    await sleep(100)
  } while (Date.now() < deadline)
  throw new Error('drill stage deadline')
}
async function send(request, variant) {
  let timeout
  try {
    const response = await Promise.race([client.send(request), new Promise((_, reject) => {
      timeout = setTimeout(() => reject(new Error('kernel request deadline')), variant === 'PromptSubmitted' ? 60000 : 10000)
    })])
    assert.ok(response?.[variant], `expected ${variant}`)
    return response[variant]
  } finally { clearTimeout(timeout) }
}
try {
  const dirs = Object.fromEntries(['home', 'config', 'data', 'cache', 'state', 'workspace', 'tmp', 'chariox'].map(name => [name, path.join(root, name)]))
  for (const dir of Object.values(dirs)) await fs.mkdir(dir, { mode: 0o700 })
  endpoint.listen(0, '127.0.0.1')
  await once(endpoint, 'listening')
  const config = { autoupdate: false, share: 'disabled', enabled_providers: ['openai'],
    model: 'openai/gpt-4.1-mini', small_model: 'openai/gpt-4.1-mini', permission: 'deny',
    provider: { openai: { options: { apiKey: 'LOCAL-DRILL-NOT-A-REAL-KEY', baseURL: `http://127.0.0.1:${endpoint.address().port}/v1` } } } }
  await fs.mkdir(path.join(dirs.config, 'opencode'), { mode: 0o700 })
  await fs.mkdir(path.join(dirs.data, 'opencode'), { mode: 0o700 })
  await fs.writeFile(path.join(dirs.config, 'opencode/opencode.json'), JSON.stringify(config), { mode: 0o600 })
  await fs.writeFile(path.join(dirs.data, 'opencode/auth.json'), JSON.stringify({ openai: { type: 'api', key: 'LOCAL-DRILL-NOT-A-REAL-KEY' } }), { mode: 0o600 })
  let launchBinary = opencodeBinary
  if (report.pure) {
    launchBinary = path.join(root, 'official-opencode-pure.sh')
    const quoted = "'" + opencodeBinary.replaceAll("'", "'\\''") + "'"
    await fs.writeFile(launchBinary, `#!/bin/sh\nexec ${quoted} --pure "$@"\n`, { mode: 0o700 })
  }
  const port = await reservePort()
  const mcp = await reservePort(), codex = await reservePort(), opencode = await reservePort()
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(kernelBinary)) hash.update(chunk)
  report.kernelSha256 = hash.digest('hex')
  report.protocol = (await execute(kernelBinary, ['--print-local-daemon-protocol-version'], { timeout: 5000 })).stdout.trim()
  report.providerVersion = (await execute(opencodeBinary, ['--version'], { timeout: 5000 })).stdout.trim()
  const repo = path.resolve(import.meta.dirname, '../../..')
  report.drillSourceHead = (await execute('git', ['rev-parse', 'HEAD'], { cwd: repo })).stdout.trim()
  report.drillSourceDirty = (await execute('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: repo })).stdout.trim().length > 0
  stage = 'kernel-start'
  kernel = spawn(kernelBinary, [], { cwd: dirs.workspace, detached: true, stdio: ['ignore', 'pipe', 'pipe'], env: {
    PATH: process.env.PATH, HOME: dirs.home, TMPDIR: dirs.tmp,
    XDG_CONFIG_HOME: dirs.config, XDG_DATA_HOME: dirs.data, XDG_CACHE_HOME: dirs.cache, XDG_STATE_HOME: dirs.state,
    OPENCODE_CONFIG_DIR: path.join(dirs.config, 'opencode'), CHARIOX_OPENCODE_BIN: launchBinary,
    CHARIOX_HOME: dirs.chariox, CHARIOX_LOG_DIR: path.join(root, 'logs'), CHARIOX_LOG_LEVEL: 'warn',
    TOKIO_WORKER_THREADS: report.workerThreads, CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcp),
    CHARIOX_CODEX_PORT: String(codex), CHARIOX_OPENCODE_PORT: String(opencode),
    CHARIOX_DAEMON_SOCKET: path.join(root, 'daemon.sock'), CHARIOX_DAEMON_ID: 'retry-drill',
    CHARIOX_MACHINE_ID: 'retry-drill-machine',
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
  } })
  for (const stream of [kernel.stdout, kernel.stderr]) stream.on('data', chunk => { startupOutput = (startupOutput + chunk.toString()).slice(-8192) })
  await waitFor(() => listening(port), 20000)
  client = new LocalIpcClient(`ws://127.0.0.1:${port}`)
  await waitFor(() => send(requests.listSessionsRequest(), 'SessionsListed').then(() => true).catch(() => false), 20000)
  stage = 'session-create'
  session = (await send(requests.createSessionRequest(dirs.workspace, dirs.workspace, 'opencode-retry', { provider: 'managed-dev-stub' }), 'SessionCreated')).session
  const attachment = (await send(requests.attachToSessionRequest(session.id, 'retry-drill'), 'SessionAttached')).attachment
  stage = 'agent-spawn'
  const agent = (await send(requests.spawnAgentRequest(session.id, 'opencode', 'retry-provider', 'openai/gpt-4.1-mini', dirs.workspace,
    'low', 'build', 'required', undefined, undefined, undefined, 'default'), 'AgentSpawned')).agent
  client.onKernelEvent(event => {
    for (const text of terminalProviderOutputSnapshot([event], agent.id).statuses) {
      const attempt = Number(text.match(/Attempt (\d+)\./)?.[1])
      if (!attempt || report.retries.some(item => item.attempt === attempt)) continue
      report.retries.push({ attempt, expectedReason: text.includes('Chariox retry drill temporarily throttled'),
        nextRetryVisible: /Next retry: \d{4}-\d\d-\d\d/.test(text), wrongNetworkReason: text.includes('connection interrupted') })
    }
  })
  await client.subscribeToKernelEvents(session.id, attachment.id)
  stage = 'prompt-submit'
  const submitStarted = Date.now()
  const submitted = await send(requests.submitPromptRequest(session.id, attachment.id, agent.id, 'Reply OK. Do not use tools.', []), 'PromptSubmitted')
  report.submitMs = Date.now() - submitStarted
  report.requestsAtSubmission = report.modelRequests
  const promptId = (submitted.outcome.Started ?? submitted.outcome.Queued)?.prompt?.id
  assert.ok(promptId)
  const processList = await send(requests.listProviderProcessesRequest('opencode'), 'ProviderProcessesListed')
  const ownedProcess = processList.processes.find(item => item.owner_session_ids.includes(session.id))
  assert.ok(ownedProcess)
  report.providerPid = ownedProcess.pid
  const run = (await send(requests.getProviderRunRequest(ownedProcess.owner_provider_run_ids[0]), 'ProviderRun')).provider_run
  const nativeUrl = new URL(run.structured_endpoint)
  assert.equal(nativeUrl.hostname, '127.0.0.1')
  ports.push(Number(nativeUrl.port))
  stage = 'retry-projection'
  await waitFor(async () => {
    const nativeStatuses = await fetch(new URL('/session/status', nativeUrl), { signal: AbortSignal.timeout(500) })
      .then(response => response.json()).catch(() => null)
    if (nativeStatuses) {
      report.nativeStatuses ??= []
      for (const status of Object.values(nativeStatuses)) {
        const sample = { type: status.type, attempt: status.attempt, expectedReason: status.message?.includes('Chariox retry drill temporarily throttled') }
        if (!report.nativeStatuses.some(item => JSON.stringify(item) === JSON.stringify(sample))) report.nativeStatuses.push(sample)
      }
    }
    const history = await send(requests.getSessionHistoryOutlineRequest(session.id, [agent.id], 2), 'SessionHistoryOutline')
    const turn = history.agents?.find(item => item.agent_id === agent.id)?.turns?.find(item => item.prompt_id === promptId)
    report.lastTurnLifecycle = turn?.lifecycle
    report.historyPolls = (report.historyPolls ?? 0) + 1
    assert.ok(!turn || turn.lifecycle !== 'completed', 'retry must not complete the turn')
    return turn?.lifecycle === 'open' && report.retries.length >= 2
  }, 60000)
  assert.ok(report.modelRequests >= 2)
  assert.ok(report.retries.every(item => item.expectedReason && item.nextRetryVisible && !item.wrongNetworkReason))
  stage = 'cancel-retry'
  const cancelledAt = Date.now()
  await send(requests.cancelActivePromptRequest(session.id, attachment.id, agent.id), 'PromptCancelled')
  await waitFor(async () => {
    const state = await send(requests.getSessionStateRequest(session.id), 'SessionState')
    return state.session.agents.find(item => item.id === agent.id)?.is_processing === false
  }, 15000)
  report.cancelRecoveryMs = Date.now() - cancelledAt
  const nativeAfterCancel = await fetch(new URL('/session/status', nativeUrl), { signal: AbortSignal.timeout(1000) }).then(response => response.json())
  report.nativeIdleAfterCancel = Object.values(nativeAfterCancel).every(status => status.type === 'idle')
  assert.ok(report.nativeIdleAfterCancel, 'native provider must be idle after cancel')
  const foregroundRequests = () => report.requestKinds.filter(item => !item.titleRequest).length
  const requestsAfterCancel = foregroundRequests()
  await sleep(2300)
  report.noForegroundRequestsAfterCancel = foregroundRequests() === requestsAfterCancel
  assert.ok(report.noForegroundRequestsAfterCancel, 'foreground model requests must stop after cancel')
  report.status = 'passed'
} catch (error) {
  failure = error
  report.status = 'failed'
  report.failureStage = stage
  report.errorType = error.name
  report.errorCodes = ['kernel request deadline', 'drill stage deadline', 'retry must not complete the turn', 'expected PromptSubmitted', 'timed out', 'authentication', 'model', 'workspace', 'MCP', 'not found', 'permission', 'account']
    .filter(code => error.message?.toLowerCase().includes(code.toLowerCase()))
  report.startupCodes = ['permission denied', 'address already in use', 'relay token', 'config', 'No such file', 'invalid', 'bind', 'refused']
    .filter(code => startupOutput.toLowerCase().includes(code.toLowerCase()))
  report.kernelExitCode = kernel?.exitCode
  report.startupVariant = startupOutput.match(/Error: ([A-Za-z]{1,64})/)?.[1]
  report.startupOperation = startupOutput.match(/operation: "([a-zA-Z ._-]{1,64})"/)?.[1]
} finally {
  const ownedChildren = kernel?.pid && alive(kernel.pid) ? await descendants(kernel.pid) : []
  if (session && client) await send(requests.endSessionRequest(session.id), 'SessionEnded').catch(() => {})
  await client?.close().catch(() => {})
  if (kernel?.pid && kernel.exitCode === null && kernel.signalCode === null) {
    const exited = once(kernel, 'exit')
    process.kill(-kernel.pid, 'SIGTERM')
    const force = setTimeout(() => { try { process.kill(-kernel.pid, 'SIGKILL') } catch {} }, 3000)
    await exited.finally(() => clearTimeout(force))
  }
  endpoint.closeAllConnections()
  await new Promise(resolve => endpoint.close(resolve))
  await fs.rm(root, { recursive: true, force: true })
  report.cleanup = { kernelExited: !kernel || !alive(kernel.pid),
    providerExited: !report.providerPid || !alive(report.providerPid),
    childrenExited: ownedChildren.every(pid => !alive(pid)),
    portsClosed: (await Promise.all(ports.map(listening))).every(value => !value),
    tempRootRemoved: await fs.access(root).then(() => false).catch(() => true), endpointClosed: !endpoint.listening }
  if (Object.values(report.cleanup).some(value => !value)) {
    failure ??= new Error('drill cleanup incomplete')
    report.status = 'failed'
  }
  await fs.mkdir(path.dirname(evidence), { recursive: true })
  await fs.writeFile(evidence, JSON.stringify(report, null, 2) + '\n', { mode: 0o600 })
}
console.log(JSON.stringify({ ...report, evidence }))
if (failure) process.exitCode = 1

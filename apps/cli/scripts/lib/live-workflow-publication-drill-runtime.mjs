import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { WebSocket } from 'ws'

import {
  rustBinaryPath,
  rustManifestPath,
} from '../../../../scripts/rust-workspace.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
export const cliRoot = path.resolve(scriptDir, '..', '..')
export const repoRoot = path.resolve(cliRoot, '..', '..')

export function logStep(name, details = null) {
  if (details == null) console.log(`[publication-drill] ${name}`)
  else console.log(`[publication-drill] ${name}`, JSON.stringify(details))
}

export function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

export function tail(value, max = 4000) {
  return typeof value === 'string' ? value.slice(-max) : ''
}

export function envFlag(name, defaultValue = false) {
  const value = process.env[name]
  if (value == null || value === '') return defaultValue
  return ['1', 'true', 'yes', 'on'].includes(value.toLowerCase())
}

export function withPublicationDrillProviderInventory(env) {
  return { ...env, ARROBA_PROVIDER_DEV_STUB: '1' }
}

export function secureGatewayPublicationEnvs(env, {
  host,
  port,
  kernelUrl,
  tls,
  humanHttp,
  websocket,
}) {
  const common = {
    ...env,
    HOST: host,
    PORT: String(port),
    ARROBA_KERNEL_URL: kernelUrl,
    ARROBA_PUBLICATION_TLS_KEY_FILE: tls.keyFile,
    ARROBA_PUBLICATION_TLS_CERT_FILE: tls.certFile,
  }
  return {
    https: {
      ...common,
      ARROBA_PUBLICATION_SESSION_ID: humanHttp.sessionId,
      ARROBA_PUBLICATION_ID: humanHttp.publicationId,
    },
    wss: {
      ...common,
      ARROBA_PUBLICATION_SESSION_ID: websocket.sessionId,
      ARROBA_PUBLICATION_ID: websocket.publicationId,
    },
  }
}

export function realDashboardOptionsFromEnv() {
  if (!envFlag('ARROBA_PUBLICATION_REAL_DASHBOARD')) return null
  const provider = process.env.ARROBA_PUBLICATION_REAL_DASHBOARD_PROVIDER || 'codex'
  return {
    provider,
    accountProfile: process.env.ARROBA_PUBLICATION_REAL_DASHBOARD_ACCOUNT || 'default',
    model: process.env.ARROBA_PUBLICATION_REAL_DASHBOARD_MODEL || (provider === 'opencode' ? 'opencode/gpt-5.4' : 'gpt-5.5'),
    effort: process.env.ARROBA_PUBLICATION_REAL_DASHBOARD_EFFORT || 'high',
    expectThinking: envFlag('ARROBA_PUBLICATION_REAL_DASHBOARD_EXPECT_THINKING', true),
    useHostProviderHome: envFlag('ARROBA_PUBLICATION_REAL_DASHBOARD_USE_HOST_PROVIDER_HOME', true),
  }
}

export const REAL_DASHBOARD_PROMPT = [
  'Generate a vibrant dashboard as a compact self-contained HTML document.',
  'The dashboard must visibly include the title text `Real Provider Workflow Dashboard`.',
  'The main dashboard element must include `data-arroba-real-provider-dashboard="true"`.',
  'Before writing the file, reason through a compact layout plan that balances mobile responsiveness, contrast, KPI cards, one chart-like visual, and a status section.',
  'Use inline CSS only; no scripts, external assets, network calls, or unrelated file inspection.',
  'This is the final workflow node: submit final workflow output as {"kind":"html","html":"<full html document>"} by calling validate_and_submit_workflow_run_output, then emit the final fenced workflow JSON block with the same output.message object.',
].join(' ')

export async function withTimeout(promise, timeoutMs, label) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

export function variant(response, key) {
  return response?.[key] ?? response
}

export function isTerminalWorkflowRunStatus(status) {
  return ['completed', 'failed', 'stopped'].includes(String(status).toLowerCase())
}

export function hasAcceptedRunMetadata(body) {
  return !!body && (body.workflow_run?.id || body.queued === true)
}

export function sseEventNames(body) {
  return [...body.matchAll(/^event: (.+)$/gm)].map((match) => match[1])
}

export function publicationStatusWatchdogs(status) {
  if (Array.isArray(status?.watchdogs)) return status.watchdogs
  if (Array.isArray(status?.schedules)) return status.schedules
  return []
}

export function publicationStatusWatchdogCount(status) {
  if (Number.isInteger(status?.watchdog_count)) return status.watchdog_count
  if (Number.isInteger(status?.schedule_count)) return status.schedule_count
  return publicationStatusWatchdogs(status).length
}

export async function readSseUntilEvent(response, expectedEvent, options = {}) {
  if (!response.body) throw new Error('SSE response did not include a readable body')
  const timeoutMs = options.timeoutMs ?? 5_000
  const maxChars = options.maxChars ?? 64 * 1024
  const deadline = Date.now() + timeoutMs
  const decoder = new TextDecoder()
  const reader = response.body.getReader()
  let body = ''
  try {
    while (Date.now() < deadline) {
      const remainingMs = Math.max(1, deadline - Date.now())
      const chunk = await withTimeout(reader.read(), remainingMs, `SSE ${expectedEvent} event`)
      if (chunk.done) {
        body += decoder.decode()
        break
      }
      body += decoder.decode(chunk.value, { stream: true })
      if (body.length > maxChars) {
        throw new Error(`SSE stream exceeded ${maxChars} characters before ${expectedEvent}`)
      }
      if (sseEventNames(body).includes(expectedEvent)) return body
    }
    throw new Error(`SSE stream ended before ${expectedEvent}: ${tail(body, 400)}`)
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

export function createWebSocketReader(socket) {
  const queue = []
  const waiters = []
  let socketError = null
  socket.on('message', (data) => {
    let parsed
    try {
      parsed = JSON.parse(data.toString())
    } catch (error) {
      socketError = error
      return
    }
    const waiter = waiters.shift()
    if (waiter) waiter(parsed)
    else queue.push(parsed)
  })
  socket.on('error', (error) => {
    socketError = error
  })
  return {
    read: async () => {
      if (socketError) throw socketError
      const queued = queue.shift()
      if (queued !== undefined) return queued
      return await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timed out waiting for websocket message')), 20_000)
        waiters.push((value) => {
          clearTimeout(timeout)
          resolve(value)
        })
      })
    },
  }
}

export async function invokePublicationWebSocket(url, input, options = {}) {
  const socket = new WebSocket(url, options)
  const reader = createWebSocketReader(socket)
  try {
    const ready = await reader.read()
    if (ready.type !== 'ready') {
      throw new Error(`expected websocket ready message, got ${JSON.stringify(ready)}`)
    }
    socket.send(JSON.stringify({
      type: 'artifact_begin',
      artifact_id: 'ws-artifact-1',
      name: 'ws-input.txt',
      mime_type: 'text/plain',
      size_bytes: 9,
    }))
    const begun = await reader.read()
    if (begun.type !== 'artifact_ack' || begun.status !== 'begun') {
      throw new Error(`expected websocket artifact begin ack, got ${JSON.stringify(begun)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_chunk', artifact_id: 'ws-artifact-1', data: 'd3MtcHVibA==' }))
    const chunk = await reader.read()
    if (chunk.type !== 'artifact_ack' || chunk.status !== 'chunk') {
      throw new Error(`expected websocket artifact chunk ack, got ${JSON.stringify(chunk)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_end', artifact_id: 'ws-artifact-1' }))
    const readyArtifact = await reader.read()
    if (readyArtifact.type !== 'artifact' || readyArtifact.status !== 'ready') {
      throw new Error(`expected websocket artifact ready message, got ${JSON.stringify(readyArtifact)}`)
    }
    socket.send(JSON.stringify({ type: 'invoke', input }))
    const accepted = await reader.read()
    if (accepted.type !== 'accepted' || (!accepted.workflow_run?.id && !accepted.queued)) {
      throw new Error(`expected websocket accepted run metadata, got ${JSON.stringify(accepted)}`)
    }
    if (!options.waitForFinal) {
      return { accepted, messages: [accepted] }
    }
    const messages = [accepted]
    const deadline = Date.now() + (options.timeoutMs ?? 30_000)
    while (Date.now() < deadline) {
      const message = await reader.read()
      messages.push(message)
      if (message.type === 'final' || message.type === 'error' || message.type === 'timeout') {
        return { accepted, messages }
      }
    }
    throw new Error(`timed out waiting for websocket final message: ${JSON.stringify(messages)}`)
  } finally {
    socket.close()
  }
}

export function websocketUrlFromHttp(url) {
  const parsed = new URL(url)
  parsed.protocol = parsed.protocol === 'https:' ? 'wss:' : 'ws:'
  return parsed.toString()
}

export async function run(command, args, options = {}) {
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
    child.on('error', (error) => resolve({ code: 1, stdout, stderr: String(error) }))
    child.on('close', (code) => resolve({ code, stdout, stderr }))
  })
}

export async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

export async function ensureDockerAvailable() {
  await runChecked('docker', ['version', '--format', '{{.Server.Version}}'])
}

export async function buildRustBinary(binaryName) {
  const manifestPath = rustManifestPath(repoRoot, binaryName)
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binaryName])
  if (result.code !== 0) {
    throw new Error(`${binaryName} build failed\n${result.stdout}\n${result.stderr}`)
  }
  return rustBinaryPath(repoRoot, binaryName)
}

export async function buildPublicationContainerImage(tag) {
  await ensureDockerAvailable()
  await runChecked('docker', [
    'build',
    '-f',
    path.join(repoRoot, 'docker/publication/Dockerfile'),
    '-t',
    tag,
    repoRoot,
  ], { env: process.env })
}

export function startPublicationContainer({
  image,
  name,
  packageDir,
  workspaceDir,
  port,
}) {
  return startProcess('docker', [
    'run',
    '--rm',
    '--name',
    name,
    '-p',
    `127.0.0.1:${port}:3000`,
    '-v',
    `${packageDir}:/publication:ro`,
    '-v',
    `${workspaceDir}:/workspace`,
    '-e',
    'ARROBA_PUBLICATION_PACKAGE=/publication',
    '-e',
    'HOST=0.0.0.0',
    '-e',
    'PORT=3000',
    image,
    'standalone',
  ], process.env, name)
}

export async function removeDockerContainer(name) {
  await run('docker', ['rm', '-f', name], { env: process.env })
}

export async function removeDockerImage(tag) {
  await run('docker', ['image', 'rm', '-f', tag], { env: process.env })
}

export async function createContainerPortablePackage(sourceDir, targetDir) {
  await rm(targetDir, { recursive: true, force: true })
  await cp(sourceDir, targetDir, { recursive: true })
  const snapshotPath = path.join(targetDir, 'workflow.snapshot.json')
  const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
  if (snapshot.source_session) {
    snapshot.source_session.workspace_id = '/workspace'
    snapshot.source_session.worktree_id = '/workspace'
  }
  for (const agent of snapshot.agents ?? []) {
    if (agent.workspace_id != null) agent.workspace_id = '/workspace'
    if (agent.worktree_id != null) agent.worktree_id = '/workspace'
  }
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
}

export async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error('could not allocate port')))
    })
    server.on('error', reject)
  })
}

export function startProcess(command, args, env, name) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(command, args, {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  child.name = name
  return child
}

export function startServeWithProviderPrompt({
  cliBinary,
  packageDir,
  port,
  kernelUrl,
  env,
  provider,
  model,
  effort,
}) {
  const script = `
set timeout 45
set cli $env(ARROBA_EXPECT_CLI_BINARY)
set package_dir $env(ARROBA_EXPECT_PUBLICATION_PACKAGE)
set port $env(ARROBA_EXPECT_PUBLICATION_PORT)
set kernel_url $env(ARROBA_EXPECT_KERNEL_URL)
set provider $env(ARROBA_EXPECT_REPLACEMENT_PROVIDER)
set model $env(ARROBA_EXPECT_REPLACEMENT_MODEL)
set effort $env(ARROBA_EXPECT_REPLACEMENT_EFFORT)
trap { catch { exec kill [exp_pid] }; exit 143 } SIGTERM
spawn -noecho $cli serve $package_dir $port --kernel-url $kernel_url
expect {
  -re {Replacement provider:} { send -- "$provider\\r" }
  timeout { puts stderr "timed out waiting for provider replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {Replacement model .*:} { send -- "$model\\r" }
  timeout { puts stderr "timed out waiting for model replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {Replacement effort .*:} { send -- "$effort\\r" }
  timeout { puts stderr "timed out waiting for effort replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {workflow gateway listening} { puts "EXPECT_SERVE_READY"; exp_continue }
  timeout { exp_continue }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
`
  return startProcess('/usr/bin/expect', ['-c', script], {
    ...env,
    ARROBA_EXPECT_CLI_BINARY: cliBinary,
    ARROBA_EXPECT_PUBLICATION_PACKAGE: packageDir,
    ARROBA_EXPECT_PUBLICATION_PORT: String(port),
    ARROBA_EXPECT_KERNEL_URL: kernelUrl,
    ARROBA_EXPECT_REPLACEMENT_PROVIDER: provider,
    ARROBA_EXPECT_REPLACEMENT_MODEL: model,
    ARROBA_EXPECT_REPLACEMENT_EFFORT: effort,
  }, 'arroba-serve-provider-override')
}

export async function stopProcess(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    }, 3_000)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
    child.kill('SIGTERM')
  })
}

export async function waitForProcessExit(child, timeoutMs = 10_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode }
  }
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`${child.name ?? 'process'} did not exit within ${timeoutMs}ms`))
    }, timeoutMs)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

export async function createSelfSignedCertificate(root) {
  const keyFile = path.join(root, 'gateway.key')
  const certFile = path.join(root, 'gateway.crt')
  const args = [
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    keyFile,
    '-out',
    certFile,
    '-subj',
    '/CN=127.0.0.1',
    '-addext',
    'subjectAltName=IP:127.0.0.1,DNS:localhost',
    '-days',
    '1',
  ]
  let result = await run('openssl', args, { cwd: root })
  if (result.code !== 0 && result.stderr.includes('addext')) {
    result = await run('openssl', args.filter((arg, index) => arg !== '-addext' && args[index - 1] !== '-addext'), { cwd: root })
  }
  if (result.code !== 0) {
    throw new Error(`openssl self-signed certificate generation failed\n${result.stdout}\n${result.stderr}`)
  }
  return { keyFile, certFile }
}

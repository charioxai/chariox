#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-cli-kernel-restart-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 49000 + Math.floor(Math.random() * 500)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[cli-restart-drill] ${name}`)
  else console.log(`[cli-restart-drill] ${name}`, JSON.stringify(details))
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

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'target/debug/chariox-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'chariox-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return binary
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
      const request = { id, action, ...fields }
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify(request)}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function stopDaemon(child) {
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

async function waitForAutomationSnapshot(automation, predicate, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send('snapshot')
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

function startDaemon(binary, env) {
  return spawn(binary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
}

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-cli-kernel-restart-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const automationSocket = path.join(os.tmpdir(), `chariox-cli-restart-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_DAEMON_ID: `cli-restart-drill-${process.pid}-${Date.now()}`,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    CHARIOX_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let cliStdout = ''
  let cliStderr = ''
  let automation = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForKernel(kernelUrl)
    log('kernel-ready', { kernelUrl })

    const cliArgs = [
      '-q',
      '/dev/null',
      'env',
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      'bun',
      path.join(repoRoot, 'apps/cli/dist/index.js'),
      '--kernel-url', kernelUrl,
      '--automation-socket', automationSocket,
      '--create-session',
      '--workspace', workspace,
      '--worktree', workspace,
      '--provider', 'dev-stub',
      '--model', 'cli-restart-model',
      '--client-id', `cli-restart-drill-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    cli.stdout.on('data', (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on('data', (chunk) => { cliStderr += chunk.toString() })
    const cliStartupFailure = new Promise((resolve) => {
      cli.once('error', (error) => resolve(error))
      cli.once('exit', (code, signal) => {
        if (code !== 0) resolve(new Error(`CLI exited before automation socket was ready: code=${code} signal=${signal ?? 'none'}`))
      })
    })
    try {
      const startupFailure = await Promise.race([
        waitForSocket(automationSocket).then(() => null),
        cliStartupFailure,
      ])
      if (startupFailure) throw startupFailure
    } catch (error) {
      throw new Error(`${error.message}\n--- cli stdout ---\n${cliStdout.slice(-4000)}\n--- cli stderr ---\n${cliStderr.slice(-4000)}`)
    }
    automation = createAutomationClient(automationSocket)
    await automation.send('ping')
    const initial = await automation.send('wait_for', { daemonDisconnected: false, timeoutMs: 5_000 })
    const sessionId = initial.session.id
    requireCondition(Boolean(sessionId), 'CLI did not attach to a session', initial)
    requireCondition(initial.session.agentCount >= 1, 'CLI did not render any agents before restart', initial)
    log('cli-attached', { sessionId, agentCount: initial.session.agentCount })

    await stopDaemon(daemon)
    daemon = null
    const disconnected = await waitForAutomationSnapshot(
      automation,
      (snapshot) => snapshot.daemonDisconnected === true && snapshot.session?.id === sessionId,
      'CLI disconnected state',
      20_000,
    )
    requireCondition(/reconnect|lost connection/i.test(disconnected.statusLine), 'CLI did not surface reconnecting status', disconnected)
    log('cli-disconnected', { statusLine: disconnected.statusLine })

    daemon = startDaemon(kernelBinary, env)
    await waitForKernel(kernelUrl)
    const reconnected = await waitForAutomationSnapshot(
      automation,
      (snapshot) => snapshot.daemonDisconnected === false && snapshot.session?.id === sessionId,
      'CLI reconnected state',
      30_000,
    )
    requireCondition(reconnected.session.agentCount >= 1, 'CLI did not restore agent state after reconnect', reconnected)
    requireCondition(!/reconnect|lost connection/i.test(reconnected.statusLine), 'CLI status still indicates reconnect after kernel returned', reconnected)
    log('cli-reconnected', { statusLine: reconnected.statusLine, agentCount: reconnected.session.agentCount })

    await automation.send('exit').catch(() => {})
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    automation?.close()
    if (cli && cli.exitCode === null && cli.signalCode === null) cli.kill('SIGTERM')
    await stopDaemon(daemon)
    if (!succeeded && options.keepArtifactsOnFailure) {
      await mkdir(rootDir, { recursive: true }).catch(() => {})
      await writeFile(path.join(rootDir, 'cli-stdout.log'), cliStdout, 'utf8').catch(() => {})
      await writeFile(path.join(rootDir, 'cli-stderr.log'), cliStderr, 'utf8').catch(() => {})
    }
    await rm(automationSocket, { force: true }).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'cli-kernel-restart',
        kernelUrl,
        workspace,
        automationSocket,
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(`[cli-restart-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})

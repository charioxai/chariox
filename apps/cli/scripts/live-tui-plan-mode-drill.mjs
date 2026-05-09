#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-tui-plan-mode-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function log(name, details) {
  if (details === undefined) console.log(`[tui-plan-mode-drill] ${name}`)
  else console.log(`[tui-plan-mode-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
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

async function ensureBuilt() {
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

async function waitForSession(client, getSessionStateRequest, sessionId, predicate, description) {
  const deadline = Date.now() + 10_000
  let lastSession = null
  while (Date.now() < deadline) {
    const response = await client.send(getSessionStateRequest(sessionId))
    const payload = response.SessionState ?? response.SessionStateLoaded ?? response
    lastSession = payload.session ?? payload
    if (predicate(lastSession)) return lastSession
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${description}\n${JSON.stringify(lastSession, null, 2)}`)
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(3000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(1000)])
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-tui-plan-mode-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const automationSocket = path.join(os.tmpdir(), `arroba-tui-plan-${process.pid}-${Date.now()}.sock`)
  const kernelPort = 52000 + Math.floor(Math.random() * 1000)
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(kernelPort + 1000),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_DAEMON_ID: `tui-plan-mode-drill-${process.pid}-${Date.now()}`,
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
    const { cliDist, kernelBinary } = await ensureBuilt()

    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForKernel(kernelUrl)
    log('kernel-ready', { kernelUrl })

    const cliArgs = [
      '-q',
      '/dev/null',
      'env',
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      'bun',
      cliDist,
      '--kernel-url', kernelUrl,
      '--automation-socket', automationSocket,
      '--create-session',
      '--workspace', workspace,
      '--worktree', workspace,
      '--provider', 'dev-stub',
      '--model', 'tui-plan-drill-model',
      '--client-id', `tui-plan-mode-drill-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    let cliStdout = ''
    let cliStderr = ''
    cli.stdout.on('data', (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on('data', (chunk) => { cliStderr += chunk.toString() })

    await waitForSocket(automationSocket).catch((error) => {
      throw new Error(`${error.message}\n--- cli stdout ---\n${cliStdout.slice(-4000)}\n--- cli stderr ---\n${cliStderr.slice(-4000)}`)
    })
    automation = createAutomationClient(automationSocket)
    await automation.send('ping')
    const snapshot = await automation.send('snapshot')
    const sessionId = snapshot.session?.id
    const agentId = snapshot.session?.focusedAgentId
    requireCondition(Boolean(sessionId && agentId), 'TUI did not attach to a focused session', snapshot)
    log('cli-ready', { sessionId, agentId })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const { getSessionStateRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)

    const initialSession = await waitForSession(
      client,
      getSessionStateRequest,
      sessionId,
      (session) => session.active_provider_run_id != null,
      'attach-time provider run',
    )
    const initialRunId = initialSession.active_provider_run_id

    const planResult = await automation.send('workspace_shell_exec', { command: 'agent mode plan' })
    requireCondition(planResult.result?.ok === true, 'TUI shell agent mode plan failed', planResult)
    const plannedSession = await waitForSession(
      client,
      getSessionStateRequest,
      sessionId,
      (session) => {
        const agent = session.agents?.find((entry) => entry.id === agentId)
        return agent?.execution_mode_override === 'plan' && session.active_provider_run_id !== initialRunId
      },
      'agent plan override and stale provider run invalidation',
    )
    requireCondition(
      plannedSession.active_provider_run_id == null,
      'mode change should clear the stale active provider run before the next prompt',
      plannedSession,
    )

    const modeResult = await automation.send('workspace_shell_exec', { command: 'agent mode' })
    requireCondition(modeResult.result?.ok === true, 'TUI shell agent mode query failed', modeResult)
    requireCondition(/mode = plan \(agent\)/.test(modeResult.result.output ?? modeResult.result.message ?? ''), 'TUI did not report agent plan mode', modeResult)

    const submitSnapshot = await automation.send('submit_prompt', { prompt: 'Reply with exactly TUI_PLAN_MODE_DRILL_DONE.' })
    requireCondition(submitSnapshot.session?.id === sessionId, 'submit prompt lost the session', submitSnapshot)
    const relaunchedSession = await waitForSession(
      client,
      getSessionStateRequest,
      sessionId,
      (session) => session.active_provider_run_id != null,
      'provider run relaunched after plan-mode prompt',
    )
    const relaunchedAgent = relaunchedSession.agents?.find((entry) => entry.id === agentId)
    requireCondition(relaunchedAgent?.execution_mode_override === 'plan', 'relaunch lost plan-mode override', relaunchedSession)

    const sessionModeResult = await automation.send('workspace_shell_exec', { command: 'session mode plan' })
    requireCondition(sessionModeResult.result?.ok === true, 'TUI shell session mode plan failed', sessionModeResult)
    const inheritResult = await automation.send('workspace_shell_exec', { command: 'agent mode inherit' })
    requireCondition(inheritResult.result?.ok === true, 'TUI shell agent mode inherit failed', inheritResult)
    const inheritedSession = await waitForSession(
      client,
      getSessionStateRequest,
      sessionId,
      (session) => {
        const agent = session.agents?.find((entry) => entry.id === agentId)
        return agent?.execution_mode_override == null && session.config_state?.values?.['agents.mode'] === 'plan'
      },
      'agent inherited session plan mode',
    )
    requireCondition(inheritedSession.config_state?.values?.['agents.mode'] === 'plan', 'session plan mode was not preserved', inheritedSession)

    log('passed', {
      sessionId,
      agentId,
      initialRunId,
      relaunchedRunId: relaunchedSession.active_provider_run_id,
    })
    await automation.send('exit').catch(() => {})
    succeeded = true
  } finally {
    automation?.close()
    await client?.close?.().catch(() => {})
    await terminateChild(cli)
    await terminateChild(daemon)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
      await rm(automationSocket, { force: true }).catch(() => {})
    } else {
      log('kept-artifacts', { rootDir })
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

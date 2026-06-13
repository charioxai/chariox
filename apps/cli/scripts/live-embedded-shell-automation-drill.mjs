#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, stat, writeFile } from 'node:fs/promises'
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
      console.log('Usage: node apps/cli/scripts/live-embedded-shell-automation-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 48000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[embedded-shell-drill] ${name}`)
  else console.log(`[embedded-shell-drill] ${name}`, JSON.stringify(details))
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

async function buildKernel() {
  const existingBinary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(existingBinary).then((info) => info.isFile()).catch(() => false)
  if (existing) return existingBinary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return existingBinary
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
      await new Promise((resolve) => setTimeout(resolve, 250))
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
      await new Promise((resolve) => setTimeout(resolve, 250))
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

function requireCondition(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-embedded-shell-automation-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  // Keep the Unix socket path short; macOS fails bind() with long paths.
  const automationSocket = path.join(os.tmpdir(), `arroba-cli-auto-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `embedded-shell-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let cliStdout = ''
  let cliStderr = ''
  let automation = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await writeFile(path.join(workspace, 'embedded-flow.arroba'), [
      'context',
      'pwd',
      'agent spawn alpha shell-drill-model as alpha',
      'agent spawn beta shell-drill-model as beta',
      'workflow new embedded-shell-flow as wf',
      'workflow node add $wf $alpha as n1',
      'workflow node add $wf $beta as n2',
      'workflow node can-complete-run $wf $n1 false',
      'workflow node can-emit-intermediate-output $wf $n1 true',
      'workflow edge add $wf $n1 $n2',
      'workflow endpoint new $wf $n1 entry',
      'workflow show $wf',
    ].join('\n'), 'utf8')

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
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
      '--model', 'shell-drill-model',
      '--client-id', `embedded-shell-drill-${process.pid}`,
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
    log('cli-automation-ready')

    const switched = await automation.send('switch_screen', { screen: 'workflow' })
    requireCondition(switched.screen === 'workflow' && switched.workflowScreenActive === true, 'workflow screen did not activate', switched)
    await automation.send('wait_for', { screen: 'workflow', timeoutMs: 5000 })

    const sourceResult = await automation.send('workspace_shell_exec', { command: 'source embedded-flow.arroba' })
    requireCondition(sourceResult.result?.ok === true, 'embedded shell source failed', sourceResult)
    const snapshot = sourceResult.snapshot
    sessionId = snapshot.session?.id ?? null
    const workflow = snapshot.workflows?.find((entry) => entry.alias === 'embedded-shell-flow')
    requireCondition(Boolean(workflow), 'embedded shell did not create workflow', snapshot)
    requireCondition(workflow.nodeCount === 2, 'workflow node count mismatch', workflow)
    requireCondition(workflow.edgeCount === 1, 'workflow edge count mismatch', workflow)
    requireCondition(workflow.endpointCount === 1, 'workflow endpoint count mismatch', workflow)
    requireCondition(snapshot.selectedWorkflowId === workflow.id, 'workflow pane did not select shell-created workflow', snapshot)
    requireCondition(snapshot.shell?.entries?.length === 1, 'shell transcript did not record source command', snapshot.shell)
    requireCondition(/@ source embedded-flow\.arroba/.test(snapshot.shell.transcript), 'shell transcript missing source command', snapshot.shell)
    requireCondition(/workspace: /.test(snapshot.shell.transcript), 'shell transcript missing context output', snapshot.shell)

    const showResult = await automation.send('workspace_shell_exec', { command: 'workflow show $wf' })
    requireCondition(showResult.result?.ok === true, 'embedded shell variable state was not preserved', showResult)
    requireCondition(showResult.snapshot.shell?.entries?.length === 2, 'shell transcript did not append second command', showResult.snapshot.shell)
    await automation.send('wait_for', { selectedWorkflowAlias: 'embedded-shell-flow', shellEntryCount: 2, timeoutMs: 5000 })

    const finalSnapshot = await automation.send('snapshot')
    requireCondition(finalSnapshot.screen === 'workflow', 'final snapshot left workflow screen', finalSnapshot)
    requireCondition(finalSnapshot.selectedWorkflow?.alias === 'embedded-shell-flow', 'final selected workflow mismatch', finalSnapshot)
    log('embedded-shell-workflow-passed', {
      sessionId,
      workflowId: finalSnapshot.selectedWorkflow.id,
      nodes: finalSnapshot.selectedWorkflow.nodeCount,
      edges: finalSnapshot.selectedWorkflow.edgeCount,
      endpoints: finalSnapshot.selectedWorkflow.endpointCount,
    })

    await automation.send('exit').catch(() => {})
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    automation?.close()
    if (sessionId) await cleanupSession(kernelUrl, sessionId).catch(() => {})
    if (cli && !cli.killed) cli.kill('SIGTERM')
    if (daemon && !daemon.killed) daemon.kill('SIGTERM')
    await new Promise((resolve) => setTimeout(resolve, 250))
    if (!succeeded && options.keepArtifactsOnFailure) {
      await mkdir(rootDir, { recursive: true }).catch(() => {})
      await writeFile(path.join(rootDir, 'cli-stdout.log'), cliStdout, 'utf8').catch(() => {})
      await writeFile(path.join(rootDir, 'cli-stderr.log'), cliStderr, 'utf8').catch(() => {})
    }
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'embedded-shell-automation',
        kernelUrl,
        sessionId,
        workspace,
        automationSocket,
      },
      log,
    })
    await rm(automationSocket, { force: true }).catch(() => {})
  }
  log('passed')
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

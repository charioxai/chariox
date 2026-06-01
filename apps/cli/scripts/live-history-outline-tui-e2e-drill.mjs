#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-history-outline-tui-e2e-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 50000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
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

async function buildRuntime() {
  const kernel = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (kernel.code !== 0) {
    throw new Error(`kernel build failed\n${kernel.stdout}\n${kernel.stderr}`)
  }
  const kernelClient = await run('pnpm', ['--filter', '@arroba/kernel-client', 'run', 'build'])
  if (kernelClient.code !== 0) {
    throw new Error(`kernel-client build failed\n${kernelClient.stdout}\n${kernelClient.stderr}`)
  }
  const toolDisplay = await run('pnpm', ['--filter', '@arroba/tool-display', 'run', 'build'])
  if (toolDisplay.code !== 0) {
    throw new Error(`tool-display build failed\n${toolDisplay.stdout}\n${toolDisplay.stderr}`)
  }
  const cli = await run('node', [path.join(cliRoot, 'scripts/build.mjs')])
  if (cli.code !== 0) {
    throw new Error(`cli build failed\n${cli.stdout}\n${cli.stderr}`)
  }
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
}

async function waitForKernel(LocalIpcClient, listSessionsRequest, kernelUrl) {
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
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once('connect', resolve)
        socket.once('error', reject)
      })
      socket.destroy()
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

function unwrap(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key]) return resp[key]
  }
  return resp
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  const exited = await Promise.race([
    new Promise((resolve) => child.once('exit', () => resolve(true))),
    sleep(5_000).then(() => false),
  ])
  if (!exited && child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForSnapshot(automation, predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send('snapshot')
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

function flattenedEntries(snapshot) {
  const direct = Array.isArray(snapshot.transcript?.entries) ? snapshot.transcript.entries : []
  const agentPaneEntries = snapshot.agentPanes && typeof snapshot.agentPanes === 'object'
    ? Object.values(snapshot.agentPanes).flatMap((entries) => Array.isArray(entries) ? entries : [])
    : []
  return [...direct, ...agentPaneEntries]
}

async function seedHistory(client, requests, workspace) {
  const created = unwrap(
    await client.send(requests.createSessionRequest(workspace, workspace, 'history-outline-tui-e2e')),
    'SessionCreated',
  )
  const session = created.session
  const agent = created.agent
  const attachment = unwrap(
    await client.send(requests.attachToSessionRequest(session.id, `history-outline-seeder-${process.pid}`)),
    'SessionAttached',
  ).attachment
  const launched = unwrap(
    await client.send({
      LaunchProviderRun: {
        session_id: session.id,
        agent_id: agent.id,
        adapter_key: 'dev-stub',
        provider: 'slow-structured',
        account_profile: 'default',
        model: 'default',
        variant: 'low',
        structured_endpoint: null,
        provider_session_id: null,
        native_tui: false,
      },
    }),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  const providerRun = launched.provider_run
  await client.send(requests.submitPromptRequest(
    session.id,
    attachment.id,
    agent.id,
    'TUI live history outline prompt',
    [],
  ))
  await sleep(25)
  await client.send(requests.appendNativeProviderOutputRequest(
    session.id,
    attachment.id,
    providerRun.id,
    'provider_output',
    'TUI live assistant detail before tool',
    'history-outline-assistant-detail',
  ))
  await client.send(requests.appendNativeProviderOutputRequest(
    session.id,
    attachment.id,
    providerRun.id,
    'provider_tool',
    'TUI live lazy blob detail from provider tool',
    'history-outline-tool',
  ))
  await client.send(requests.appendNativeProviderOutputRequest(
    session.id,
    attachment.id,
    providerRun.id,
    'provider_output',
    'TUI live history final summary',
    'history-outline-final',
  ))
  await client.send(requests.completePromptRequest(session.id))
  return { session, agent, attachment, providerRun }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-history-outline-tui-e2e-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const configRoot = path.join(rootDir, 'config')
  const stateRoot = path.join(rootDir, 'state')
  const automationSocket = path.join(os.tmpdir(), `arroba-history-outline-tui-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configRoot,
    XDG_STATE_HOME: stateRoot,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `history-outline-tui-e2e-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history-jsonl'),
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
    await mkdir(path.join(configRoot, 'arroba'), { recursive: true })
    await mkdir(stateRoot, { recursive: true })
    await writeFile(path.join(configRoot, 'arroba', 'config.toml'), [
      'version = 1',
      '',
      '[history.archive]',
      'mode = "disabled"',
      '',
    ].join('\n'), 'utf8')

    const kernelBinary = await buildRuntime()
    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    const seeded = await seedHistory(client, requests, workspace)

    const outlineBeforeTui = unwrap(
      await client.send(requests.getSessionHistoryOutlineRequest(seeded.session.id, [seeded.agent.id], 4)),
      'SessionHistoryOutline',
    )
    assert.equal(outlineBeforeTui.agents.length, 1)
    assert.equal(outlineBeforeTui.agents[0].turns.length, 1)
    assert.equal(outlineBeforeTui.agents[0].turns[0].blobs.length >= 1, true)
    assert.equal(outlineBeforeTui.agents[0].turns[0].entries.length >= 1, true)

    let cliStdout = ''
    let cliStderr = ''
    const cliArgs = [
      '-q',
      '/dev/null',
      'env',
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      'bun',
      path.join(repoRoot, 'apps/cli/dist/index.js'),
      '--kernel-url', kernelUrl,
      '--automation-socket', automationSocket,
      '--session', seeded.session.id,
      '--workspace', workspace,
      '--worktree', workspace,
      '--provider', 'dev-stub',
      '--model', 'history-outline-e2e-model',
      '--client-id', `history-outline-tui-e2e-${process.pid}`,
    ]
    cli = spawn('script', cliArgs, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    cli.stdout.on('data', (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on('data', (chunk) => { cliStderr += chunk.toString() })
    try {
      await waitForSocket(automationSocket)
    } catch (error) {
      throw new Error(`${error.message}\n--- cli stdout ---\n${cliStdout.slice(-4000)}\n--- cli stderr ---\n${cliStderr.slice(-4000)}`)
    }
    automation = createAutomationClient(automationSocket)
    await automation.send('ping')

    const initial = await waitForSnapshot(
      automation,
      (snapshot) => flattenedEntries(snapshot).some((entry) => entry.historyBlobId),
      'TUI lazy history blob placeholder',
    )
    const initialEntries = flattenedEntries(initial)
    const placeholder = initialEntries.find((entry) => entry.historyBlobId)
    assert.equal(Boolean(placeholder), true)
    assert.equal(placeholder.historyBlobLoaded, false)
    assert.equal(initialEntries.some((entry) => entry.text === 'TUI live history outline prompt'), true)
    assert.equal(initialEntries.some((entry) => entry.text === 'TUI live assistant detail before tool'), true)
    assert.equal(initialEntries.some((entry) => entry.text === 'TUI live history final summary'), true)
    assert.equal(initialEntries.some((entry) => entry.text === 'TUI live lazy blob detail from provider tool'), false)

    await automation.send('toggle_blob', { entryId: placeholder.id, collapsed: false })
    const expanded = await waitForSnapshot(
      automation,
      (snapshot) => {
        const entries = flattenedEntries(snapshot)
        return entries.some((entry) => entry.text === 'TUI live lazy blob detail from provider tool')
          && !entries.some((entry) => entry.historyBlobId === placeholder.historyBlobId)
      },
      'TUI expanded lazy history blob content',
    )
    const expandedEntries = flattenedEntries(expanded)

    const result = {
      drill: 'history-outline-tui-e2e',
      sessionId: seeded.session.id,
      agentId: seeded.agent.id,
      outlinePromptCount: outlineBeforeTui.agents[0].turns.length,
      outlineAssistantEntryCount: outlineBeforeTui.agents[0].turns[0].entries.length,
      outlineBlobCount: outlineBeforeTui.agents[0].turns[0].blobs.length,
      initialEntryCount: initialEntries.length,
      initialPlaceholder: {
        id: placeholder.id,
        historyBlobId: placeholder.historyBlobId,
        historyBlobLoaded: placeholder.historyBlobLoaded,
        blobTitle: placeholder.blobTitle,
        blobSummary: placeholder.blobSummary,
      },
      initialHadPrompt: initialEntries.some((entry) => entry.text === 'TUI live history outline prompt'),
      initialHadAssistantDetail: initialEntries.some((entry) => entry.text === 'TUI live assistant detail before tool'),
      initialHadSummary: initialEntries.some((entry) => entry.text === 'TUI live history final summary'),
      initialHadToolDetail: initialEntries.some((entry) => entry.text === 'TUI live lazy blob detail from provider tool'),
      expandedEntryCount: expandedEntries.length,
      expandedHadToolDetail: expandedEntries.some((entry) => entry.text === 'TUI live lazy blob detail from provider tool'),
      expandedPlaceholderRemoved: !expandedEntries.some((entry) => entry.historyBlobId === placeholder.historyBlobId),
    }
    console.log(JSON.stringify(result, null, 2))
    succeeded = true
  } finally {
    automation?.close()
    await client?.close?.().catch(() => {})
    await stopChild(cli)
    await stopChild(daemon)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`[history-outline-tui-e2e] artifacts kept at ${rootDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})

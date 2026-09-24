#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { captureDrillCRoomCheckpoint } from './tui-web-parity-visual-control.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const defaultLatestManifest = path.join(repoRoot, 'target', 'live-tui-web-parity-visual-session', 'latest.json')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function parseArgs(argv) {
  const options = {
    rootDir: null,
    manifestPath: defaultLatestManifest,
    manifestPathProvided: false,
    preserve: false,
    roomSessionId: null,
    kernelUrl: null,
    sliceId: null,
    workspace: null,
    worktree: null,
    webObservationPath: null,
    relayUrl: null,
    targetDaemonId: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const next = () => {
      const value = argv[index + 1]
      if (!value) throw new Error(`missing value for ${arg}`)
      index += 1
      return value
    }
    if (arg === '--root-dir') options.rootDir = path.resolve(next())
    else if (arg === '--manifest') {
      options.manifestPath = path.resolve(next())
      options.manifestPathProvided = true
    }
    else if (arg === '--observe-room-session') options.roomSessionId = next()
    else if (arg === '--kernel-url') options.kernelUrl = next()
    else if (arg === '--slice-id') options.sliceId = next()
    else if (arg === '--workspace') options.workspace = path.resolve(next())
    else if (arg === '--worktree') options.worktree = path.resolve(next())
    else if (arg === '--web-observation') options.webObservationPath = path.resolve(next())
    else if (arg === '--relay-url') options.relayUrl = next()
    else if (arg === '--target-daemon-id') options.targetDaemonId = next()
    else if (arg === '--preserve') options.preserve = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-tui-web-parity-visual-session.mjs [--manifest PATH] [--root-dir DIR] [--preserve]\n       node apps/cli/scripts/live-tui-web-parity-visual-session.mjs --observe-room-session ID --kernel-url URL --slice-id ID --workspace PATH --worktree PATH --web-observation PATH --relay-url URL --target-daemon-id ID [--root-dir DIR] [--manifest PATH] (set CHARIOX_DRILL_C_RELAY_TOKEN)')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  assert.equal(Boolean(options.relayUrl), Boolean(options.targetDaemonId), '--relay-url and --target-daemon-id must be supplied together')
  return options
}

export function validateRemoteRelay(options, relayToken) {
  assert.ok(options.relayUrl, '--relay-url is required for the remote Room observer')
  assert.ok(options.targetDaemonId?.trim(), '--target-daemon-id is required for the remote Room observer')
  assert.ok(typeof relayToken === 'string' && relayToken.trim().length > 0, 'CHARIOX_DRILL_C_RELAY_TOKEN is required for the remote Room observer')

  let endpoint
  try {
    endpoint = new URL(options.relayUrl)
  } catch {
    throw new Error('--relay-url must be a valid WebSocket URL')
  }
  assert.ok(['ws:', 'wss:'].includes(endpoint.protocol), '--relay-url must be a WebSocket URL')
  const host = endpoint.hostname.toLowerCase().replace(/^\[|\]$/g, '')
  const ipv4 = host.split('.').map((part) => Number(part))
  const isLoopbackIpv4 = ipv4.length === 4
    && ipv4.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
    && ipv4[0] === 127
  assert.ok(host === 'localhost' || host === '::1' || isLoopbackIpv4, '--relay-url must target a same-host loopback relay')
  assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash, '--relay-url must not include credentials, query, or fragment data')

  return { url: options.relayUrl, targetDaemonId: options.targetDaemonId }
}

export function buildObserverCliArgs({
  cliPath,
  kernelUrl,
  relay,
  automationSocket,
  sessionId,
  workspace,
  worktree,
  clientId,
}) {
  const args = [cliPath]
  if (relay) {
    args.push('--relay-url', relay.url, '--relay-token-env', 'CHARIOX_DRILL_C_RELAY_TOKEN', '--target-daemon-id', relay.targetDaemonId)
  } else {
    args.push('--kernel-url', kernelUrl)
  }
  args.push(
    '--automation-socket', automationSocket,
    '--session', sessionId,
    '--workspace', workspace,
    '--worktree', worktree,
    '--client-id', clientId,
  )
  return args
}

export function assertSameRoomObservers(localSnapshot, remoteSnapshot, sessionId) {
  assert.equal(localSnapshot?.session?.id, sessionId, 'local direct-kernel TUI did not attach to the requested Room')
  assert.equal(remoteSnapshot?.session?.id, sessionId, 'remote relay TUI did not attach to the requested Room')
  assert.equal(remoteSnapshot.session.id, localSnapshot.session.id, 'local and remote TUI observers are attached to different Rooms')
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
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'chariox-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel')
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

async function readAutomationSnapshot(socketPath) {
  const requestId = `drill-c-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`
  return await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    socket.setEncoding('utf8')
    let buffer = ''
    let settled = false
    const finish = (error, snapshot) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      socket.destroy()
      if (error) reject(error)
      else resolve(snapshot)
    }
    const timer = setTimeout(() => finish(new Error('automation snapshot request timed out')), 3_000)
    socket.once('connect', () => {
      socket.write(`${JSON.stringify({ id: requestId, action: 'snapshot' })}\n`)
    })
    socket.on('data', (chunk) => {
      buffer += chunk
      const newline = buffer.indexOf('\n')
      if (newline < 0) return
      try {
        const response = JSON.parse(buffer.slice(0, newline))
        if (response.id !== requestId || response.ok !== true) {
          finish(new Error('automation snapshot request failed'))
          return
        }
        finish(null, response.data)
      } catch {
        finish(new Error('automation snapshot response was invalid'))
      }
    })
    socket.once('error', () => finish(new Error('automation socket is not ready')))
    socket.once('close', () => {
      if (!settled) finish(new Error('automation socket closed before its snapshot response'))
    })
  })
}

async function waitForObserverRoom(socketPath, sessionId, label, child) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`${label} TUI exited before attaching to the requested Room`)
    }
    try {
      const snapshot = await readAutomationSnapshot(socketPath)
      if (snapshot?.session?.id === sessionId) return snapshot
      if (snapshot?.session?.id) throw new Error(`${label} TUI attached to an unexpected Room`)
    } catch (error) {
      if (error.message === `${label} TUI attached to an unexpected Room`) throw error
    }
    await sleep(250)
  }
  throw new Error(`${label} TUI did not attach to the requested Room before timeout`)
}

function waitForObserverChildren(children) {
  for (const { label, child } of children) {
    if (child.exitCode !== null || child.signalCode !== null) {
      return Promise.reject(new Error(`${label} TUI exited before the observer session completed`))
    }
  }
  return new Promise((resolve, reject) => {
    const fail = (label, reason) => reject(new Error(`${label} TUI observer exited: ${reason}`))
    for (const { label, child } of children) {
      child.once('error', () => fail(label, 'process error'))
      child.once('exit', (code, signal) => fail(label, `code=${code} signal=${signal ?? 'none'}`))
    }
  })
}

function unwrap(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key]) return resp[key]
  }
  return resp
}

function promptFromOutcome(outcome) {
  return outcome?.Started?.prompt ?? outcome?.Queued?.prompt ?? outcome?.prompt ?? null
}

async function submitPrompt(client, requests, sessionId, attachmentId, agentId, prompt, attachments = []) {
  const payload = unwrap(
    await client.send(requests.submitPromptRequest(sessionId, attachmentId, agentId, prompt, attachments)),
    'PromptSubmitted',
  )
  const queuedPrompt = promptFromOutcome(payload.outcome)
  if (!queuedPrompt?.id) {
    throw new Error(`submit prompt did not return a prompt id: ${JSON.stringify(payload.outcome)}`)
  }
  return { payload, prompt: queuedPrompt }
}

async function appendOutput(client, requests, sessionId, attachmentId, providerRunId, kind, text, mergeKey) {
  await client.send(requests.appendNativeProviderOutputRequest(
    sessionId,
    attachmentId,
    providerRunId,
    kind,
    text,
    mergeKey,
  ))
}

async function seedCompletedTurn(client, requests, seeded, name, promptText, options = {}) {
  await submitPrompt(client, requests, seeded.session.id, seeded.attachment.id, seeded.agent.id, promptText, [])
  await sleep(40)
  await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_output', `${name} assistant message expanded by default.`, `${name}-assistant`)
  await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_reasoning', `${name} reasoning blob should start collapsed.`, `${name}-reasoning`)
  await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_tool', `${name} tool blob should start collapsed.`, `${name}-tool`)
  await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_status', `${name} status blob should start collapsed.`, `${name}-status`)
  if (options.includeError) {
    await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_error', `${name} error entry should stay expanded.`, `${name}-error`)
  }
  await appendOutput(client, requests, seeded.session.id, seeded.attachment.id, seeded.providerRun.id, 'provider_output', `${name} final assistant summary.`, `${name}-summary`)
  await client.send(requests.completePromptRequest(seeded.session.id))
  await sleep(60)
}

async function seedVisualSession(client, requests, workspace) {
  const created = unwrap(
    await client.send(requests.createSessionRequest(workspace, workspace, 'tui-web-parity-visual', {
      provider: 'dev-stub',
      model: 'native-tui-idle',
      effort: 'low',
      account_profile: 'default',
      execution_mode: 'build',
      permission_level: 'yolo',
    })),
    'SessionCreated',
  )
  const session = created.session
  const agent = created.agent
  await client.send(requests.aliasAgentRequest(session.id, agent.id, 'parity-agent'))
  const attachment = unwrap(
    await client.send(requests.attachToSessionRequest(session.id, `tui-web-parity-seeder-${process.pid}`)),
    'SessionAttached',
  ).attachment
  const launched = unwrap(
    await client.send({
      LaunchProviderRun: {
        session_id: session.id,
        agent_id: agent.id,
        adapter_key: 'dev-stub',
        provider: 'dev-stub',
        account_profile: 'default',
        model: 'native-tui-idle',
        variant: 'low',
        structured_endpoint: null,
        provider_session_id: null,
        native_tui: false,
      },
    }),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  const seeded = { session, agent, attachment, providerRun: launched.provider_run }

  await seedCompletedTurn(client, requests, seeded, 'historical-turn-one', 'Historical user prompt one for TUI visual parity.')
  await seedCompletedTurn(client, requests, seeded, 'historical-turn-two', 'Historical user prompt two with an error entry.', { includeError: true })

  await submitPrompt(client, requests, session.id, attachment.id, agent.id, 'Active latest user prompt should remain expanded while running.', [])
  await sleep(40)
  await appendOutput(client, requests, session.id, attachment.id, seeded.providerRun.id, 'provider_reasoning', 'Active latest reasoning blob should be collapsed but the active turn should remain expanded.', 'active-reasoning')
  await appendOutput(client, requests, session.id, attachment.id, seeded.providerRun.id, 'provider_tool', 'Active latest tool blob should be collapsed and visually distinct.', 'active-tool')
  await appendOutput(client, requests, session.id, attachment.id, seeded.providerRun.id, 'provider_status', 'Active latest status blob should be collapsed and visually distinct.', 'active-status')
  await appendOutput(client, requests, session.id, attachment.id, seeded.providerRun.id, 'provider_output', 'Active latest assistant message should be visible and readable.', 'active-assistant')

  const attachmentPart = {
    url: 'chariox-test://queued-attachment.txt',
    mime: 'text/plain',
    filename: 'queued-attachment.txt',
    contents_base64: Buffer.from('queued prompt attachment for TUI visual parity', 'utf8').toString('base64'),
  }
  const steerable = await submitPrompt(
    client,
    requests,
    session.id,
    attachment.id,
    agent.id,
    'Queued prompt one should be steerable from the visible strip.',
    [attachmentPart],
  )
  const cancellable = await submitPrompt(
    client,
    requests,
    session.id,
    attachment.id,
    agent.id,
    'Queued prompt two should be cancellable from the visible strip.',
    [],
  )
  await sleep(80)

  return {
    sessionId: session.id,
    agentId: agent.id,
    attachmentId: attachment.id,
    providerRunId: seeded.providerRun.id,
    queuedPromptIds: {
      steerable: steerable.prompt.id,
      cancellable: cancellable.prompt.id,
    },
  }
}

async function seedWaitingRoomSessions(client, requests, workspace) {
  const idle = unwrap(
    await client.send(requests.createSessionRequest(workspace, workspace, 'wr-idle-parity', {
      provider: 'dev-stub',
      model: 'native-tui-idle',
    })),
    'SessionCreated',
  ).session

  const doneCreated = unwrap(
    await client.send(requests.createSessionRequest(workspace, workspace, 'wr-done-parity', {
      provider: 'dev-stub',
      model: 'native-tui-idle',
    })),
    'SessionCreated',
  )
  const doneSession = doneCreated.session
  const doneAgent = doneCreated.agent
  const doneAttachment = unwrap(
    await client.send(requests.attachToSessionRequest(doneSession.id, `tui-web-parity-done-${process.pid}`)),
    'SessionAttached',
  ).attachment
  const doneRun = unwrap(
    await client.send({
      LaunchProviderRun: {
        session_id: doneSession.id,
        agent_id: doneAgent.id,
        adapter_key: 'dev-stub',
        provider: 'dev-stub',
        account_profile: 'default',
        model: 'native-tui-idle',
        variant: null,
        structured_endpoint: null,
        provider_session_id: null,
        native_tui: false,
      },
    }),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  ).provider_run
  await submitPrompt(client, requests, doneSession.id, doneAttachment.id, doneAgent.id, 'Waiting room done prompt.', [])
  await appendOutput(client, requests, doneSession.id, doneAttachment.id, doneRun.id, 'provider_output', 'Waiting room done assistant output.', 'wr-done-output')
  await client.send(requests.completePromptRequest(doneSession.id))

  let sliceId = null
  if (typeof requests.createSliceRequest === 'function') {
    const slice = unwrap(
      await client.send(requests.createSliceRequest({
        name: 'wr-slice-next-action',
        displayMode: 'headless',
        workspaceId: workspace,
        worktreeId: workspace,
      })),
      'SliceCreated',
    ).slice
    sliceId = slice?.id ?? null
  }

  return {
    idleSessionId: idle.id,
    doneSessionId: doneSession.id,
    sliceId,
  }
}

async function writeManifest(manifestPath, manifest) {
  await mkdir(path.dirname(manifestPath), { recursive: true })
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
}

export function buildRoomObserverManifest({
  startedAt,
  repoRoot,
  rootDir,
  evidenceDir,
  kernelUrl,
  sessionId,
  sliceId,
  workspace,
  worktree,
  webObservationPath,
  baseline,
  relay,
  automationSocket,
  remoteAutomationSocket,
  reportPath,
  manifestPath,
}) {
  assert.ok(path.isAbsolute(automationSocket), 'local automation socket must be absolute')
  assert.ok(path.isAbsolute(remoteAutomationSocket), 'remote automation socket must be absolute')
  assert.notEqual(automationSocket, remoteAutomationSocket, 'local and remote observers require distinct automation sockets')
  return {
    schema: 'chariox.drill_c.live_observer_session.v1',
    startedAt,
    repoRoot,
    rootDir,
    evidenceDir,
    kernelUrl,
    sessionId,
    sliceId,
    workspace,
    worktree,
    webObservationPath,
    baseline,
    webObserverContract: {
      schema: 'chariox.browser_computer.drill_c.web_observer.v1',
      required: [
        'sessionId, environmentId, sliceId, runtimeGeneration',
        'display.viewport, display.browserIds, display.profileIds',
        'browser.focusedTabId and browser.tab { tabId, url, title, documentRevision }',
        'actions.computer and actions.webTakeover with actionId, actorId, sequence, mode, kind, state',
      ],
    },
    automationSocket,
    remoteAutomationSocket,
    remoteRelay: {
      url: relay.url,
      targetDaemonId: relay.targetDaemonId,
    },
    reportPath,
    command: {
      verify: `node apps/cli/scripts/tui-web-parity-visual-control.mjs --manifest ${manifestPath} --action drill-c-verify`,
    },
    cleanup: 'observer TUIs and local IPC connection only; the Room, slice, and session are preserved',
  }
}

export function observerSocketPaths(stamp, temporaryDirectory = os.tmpdir()) {
  const socket = (suffix) => {
    const name = `chariox-dc-${stamp}-${suffix}.sock`
    const preferred = path.join(temporaryDirectory, name)
    return Buffer.byteLength(preferred) <= 100 ? preferred : path.join('/tmp', name)
  }
  return { local: socket('l'), remote: socket('r') }
}

async function runRoomObserverSession(options) {
  for (const [name, flag] of [
    ['roomSessionId', '--observe-room-session'],
    ['kernelUrl', '--kernel-url'],
    ['sliceId', '--slice-id'],
    ['workspace', '--workspace'],
    ['worktree', '--worktree'],
    ['webObservationPath', '--web-observation'],
  ]) {
    assert.ok(options[name], `${flag} is required with --observe-room-session`)
  }
  assert.ok(path.isAbsolute(options.workspace) && path.isAbsolute(options.worktree), 'Room workspace and worktree must be absolute')
  assert.ok(path.isAbsolute(options.webObservationPath), 'Web observation path must be absolute')
  const kernelEndpoint = new URL(options.kernelUrl)
  assert.ok(['ws:', 'wss:'].includes(kernelEndpoint.protocol), '--kernel-url must be a WebSocket URL')
  const relayToken = process.env.CHARIOX_DRILL_C_RELAY_TOKEN
  const remoteRelay = validateRemoteRelay(options, relayToken)

  const stamp = `${process.pid}-${Date.now()}`
  const rootDir = options.rootDir ?? path.join(os.homedir(), '.chariox', 'dev', 'browser-computer-use', `drill-c-live-observer-${stamp}`)
  assert.ok(rootDir !== repoRoot && !rootDir.startsWith(`${repoRoot}${path.sep}`), 'observer artifacts must be outside the repository')
  const evidenceDir = path.join(rootDir, 'evidence')
  const manifestPath = options.manifestPathProvided ? options.manifestPath : path.join(rootDir, 'manifest.json')
  assert.ok(manifestPath !== repoRoot && !manifestPath.startsWith(`${repoRoot}${path.sep}`), 'observer manifest must be outside the repository')
  assert.notEqual(manifestPath, options.webObservationPath, 'observer manifest and Web report must use separate files')
  const { local: automationSocket, remote: remoteAutomationSocket } = observerSocketPaths(stamp)
  assert.ok(path.isAbsolute(remoteAutomationSocket), 'remote automation socket must be absolute')
  assert.notEqual(automationSocket, remoteAutomationSocket, 'local and remote observers require distinct automation sockets')
  await mkdir(evidenceDir, { recursive: true, mode: 0o700 })

  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(options.kernelUrl)
  let localCli = null
  let remoteCli = null
  let cleaned = false
  const cleanup = async () => {
    if (cleaned) return
    cleaned = true
    await client.close?.().catch(() => {})
    await stopChild(remoteCli)
    await stopChild(localCli)
    await rm(automationSocket, { force: true }).catch(() => {})
    await rm(remoteAutomationSocket, { force: true }).catch(() => {})
  }

  process.once('SIGINT', () => { void cleanup().then(() => process.exit(130)) })
  process.once('SIGTERM', () => { void cleanup().then(() => process.exit(143)) })

  try {
    const baseline = await captureDrillCRoomCheckpoint({
      client,
      requests,
      sessionId: options.roomSessionId,
      sliceId: options.sliceId,
    })
    const manifest = buildRoomObserverManifest({
      startedAt: new Date().toISOString(),
      repoRoot,
      rootDir,
      evidenceDir,
      kernelUrl: options.kernelUrl,
      sessionId: options.roomSessionId,
      sliceId: options.sliceId,
      workspace: options.workspace,
      worktree: options.worktree,
      webObservationPath: options.webObservationPath,
      baseline,
      relay: remoteRelay,
      automationSocket,
      remoteAutomationSocket,
      reportPath: path.join(evidenceDir, 'drill-c-live-observation.json'),
      manifestPath,
    })
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, { encoding: 'utf8', mode: 0o600, flag: 'wx' })
    console.log(`[tui-web-parity-visual] live Room observer manifest: ${manifestPath}`)
    console.log('[tui-web-parity-visual] attached without creating or mutating the Room; start before Computer work, then run drill-c-verify after Web takeover')

    const cliPath = path.join(repoRoot, 'apps/cli/dist/index.js')
    const observerEnv = { ...process.env }
    delete observerEnv.CHARIOX_DRILL_C_RELAY_TOKEN
    localCli = spawn('bun', buildObserverCliArgs({
      cliPath,
      kernelUrl: options.kernelUrl,
      automationSocket,
      sessionId: options.roomSessionId,
      workspace: options.workspace,
      worktree: options.worktree,
      clientId: `chariox-drill-c-observer-${stamp}-local`,
    }), {
      cwd: repoRoot,
      env: observerEnv,
      stdio: 'inherit',
    })
    const localStartupFailure = new Promise((resolve) => {
      localCli.once('error', () => resolve(new Error('local direct-kernel observer CLI failed to start')))
      localCli.once('exit', (code, signal) => resolve(new Error(`local direct-kernel observer CLI exited before Room attachment: code=${code} signal=${signal ?? 'none'}`)))
    })
    const localStartup = await Promise.race([
      waitForObserverRoom(automationSocket, options.roomSessionId, 'local direct-kernel', localCli).then((snapshot) => ({ snapshot })),
      localStartupFailure.then((error) => ({ error })),
    ])
    if (localStartup.error) throw localStartup.error

    remoteCli = spawn('bun', buildObserverCliArgs({
      cliPath,
      relay: remoteRelay,
      automationSocket: remoteAutomationSocket,
      sessionId: options.roomSessionId,
      workspace: options.workspace,
      worktree: options.worktree,
      clientId: `chariox-drill-c-observer-${stamp}-remote`,
    }), {
      cwd: repoRoot,
      env: { ...observerEnv, CHARIOX_DRILL_C_RELAY_TOKEN: relayToken },
      stdio: 'inherit',
    })
    const remoteStartupFailure = new Promise((resolve) => {
      remoteCli.once('error', () => resolve(new Error('remote relay observer CLI failed to start')))
      remoteCli.once('exit', (code, signal) => resolve(new Error(`remote relay observer CLI exited before Room attachment: code=${code} signal=${signal ?? 'none'}`)))
    })
    const remoteStartup = await Promise.race([
      waitForObserverRoom(remoteAutomationSocket, options.roomSessionId, 'remote relay', remoteCli).then((snapshot) => ({ snapshot })),
      remoteStartupFailure.then((error) => ({ error })),
    ])
    if (remoteStartup.error) throw remoteStartup.error
    assertSameRoomObservers(localStartup.snapshot, remoteStartup.snapshot, options.roomSessionId)
    console.log('[tui-web-parity-visual] local direct-kernel and remote relay TUIs both attached to the requested Room')
    await waitForObserverChildren([
      { label: 'local direct-kernel', child: localCli },
      { label: 'remote relay', child: remoteCli },
    ])
  } finally {
    await cleanup()
  }
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const observerModeRequested = [
    options.roomSessionId,
    options.kernelUrl,
    options.sliceId,
    options.workspace,
    options.worktree,
    options.webObservationPath,
    options.relayUrl,
    options.targetDaemonId,
  ].some(Boolean)
  if (observerModeRequested) {
    await runRoomObserverSession(options)
    return
  }
  const rootDir = options.rootDir ?? path.join(repoRoot, 'target', 'live-tui-web-parity-visual-session', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const configRoot = path.join(rootDir, 'config')
  const stateRoot = path.join(rootDir, 'state')
  const evidenceDir = path.join(rootDir, 'evidence')
  const automationSocket = path.join(os.tmpdir(), `chariox-tui-web-parity-visual-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const daemonId = `tui-web-parity-visual-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configRoot,
    XDG_STATE_HOME: stateRoot,
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'history-jsonl'),
    CHARIOX_TEST_TUI: '1',
  }

  let daemon = null
  let cli = null
  let client = null
  let cleaned = false
  const cleanup = async () => {
    if (cleaned) return
    cleaned = true
    await client?.close?.().catch(() => {})
    await stopChild(cli)
    await stopChild(daemon)
    await rm(automationSocket, { force: true }).catch(() => {})
    if (!options.preserve) {
      await writeFile(path.join(rootDir, 'CLEANED_UP'), new Date().toISOString(), 'utf8').catch(() => {})
    }
  }

  process.once('SIGINT', () => {
    void cleanup().then(() => process.exit(130))
  })
  process.once('SIGTERM', () => {
    void cleanup().then(() => process.exit(143))
  })

  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configRoot, 'chariox'), { recursive: true })
    await mkdir(stateRoot, { recursive: true })
    await mkdir(evidenceDir, { recursive: true })
    await writeFile(path.join(configRoot, 'chariox', 'config.toml'), [
      'version = 1',
      '',
      '[history.archive]',
      'mode = "disabled"',
      '',
    ].join('\n'), 'utf8')

    const kernelBinary = await buildKernel()
    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env,
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    const visual = await seedVisualSession(client, requests, workspace)
    const waitingRoom = await seedWaitingRoomSessions(client, requests, workspace)

    const manifest = {
      schema: 'chariox.tui_web_parity_visual_session.v1',
      startedAt: new Date().toISOString(),
      repoRoot,
      rootDir,
      workspace,
      evidenceDir,
      reportPath: path.join(evidenceDir, 'visual-validation-report.json'),
      kernelUrl,
      automationSocket,
      daemonId,
      ...visual,
      waitingRoom,
      drillCSharedBrowserComputer: {
        status: 'not_proven',
        reason: 'this dev-stub visual session does not create a Room Environment or observe Web takeover',
      },
      command: {
        control: `pnpm --filter @chariox/cli run tui-web-parity:visual-control --manifest ${options.manifestPath}`,
      },
    }
    await writeManifest(options.manifestPath, manifest)
    await writeManifest(path.join(rootDir, 'manifest.json'), manifest)
    console.log(`[tui-web-parity-visual] manifest: ${options.manifestPath}`)
    console.log('[tui-web-parity-visual] launching visible TUI; use /waiting and /exit from the TUI when validating manually')

    cli = spawn('bun', [
      path.join(repoRoot, 'apps/cli/dist/index.js'),
      '--kernel-url', kernelUrl,
      '--automation-socket', automationSocket,
      '--session', visual.sessionId,
      '--workspace', workspace,
      '--worktree', workspace,
      '--provider', 'dev-stub',
      '--model', 'native-tui-idle',
      '--effort', 'low',
      '--client-id', `tui-web-parity-visual-${process.pid}`,
    ], {
      cwd: repoRoot,
      env,
      stdio: 'inherit',
    })
    const startupFailure = new Promise((resolve) => {
      cli.once('error', (error) => resolve(error))
      cli.once('exit', (code, signal) => {
        if (code !== 0) resolve(new Error(`CLI exited before automation socket was ready: code=${code} signal=${signal ?? 'none'}`))
      })
    })
    const failed = await Promise.race([
      waitForSocket(automationSocket).then(() => null),
      startupFailure,
    ])
    if (failed) throw failed

    await new Promise((resolve, reject) => {
      cli.once('error', reject)
      cli.once('exit', (code, signal) => {
        if (signal) reject(new Error(`CLI exited by signal ${signal}`))
        else if (code && code !== 0) reject(new Error(`CLI exited with code ${code}`))
        else resolve()
      })
    })
  } finally {
    await cleanup()
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`[tui-web-parity-visual] ${error.stack ?? error.message}`)
    process.exitCode = 1
  })
}

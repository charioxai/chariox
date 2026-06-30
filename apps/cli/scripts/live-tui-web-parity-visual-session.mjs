#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const defaultLatestManifest = path.join(repoRoot, 'target', 'live-tui-web-parity-visual-session', 'latest.json')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function parseArgs(argv) {
  const options = {
    rootDir: null,
    manifestPath: defaultLatestManifest,
    preserve: false,
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
    else if (arg === '--manifest') options.manifestPath = path.resolve(next())
    else if (arg === '--preserve') options.preserve = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-tui-web-parity-visual-session.mjs [--manifest PATH] [--root-dir DIR] [--preserve]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
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
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
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
  await appendOutput(client, requests, session.id, attachment.id, seeded.providerRun.id, 'provider_output', 'Active latest assistant message should be visible and readable.', 'active-assistant')

  const attachmentPart = {
    url: 'arroba-test://queued-attachment.txt',
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
  const rootDir = options.rootDir ?? path.join(repoRoot, 'target', 'live-tui-web-parity-visual-session', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const configRoot = path.join(rootDir, 'config')
  const stateRoot = path.join(rootDir, 'state')
  const evidenceDir = path.join(rootDir, 'evidence')
  const automationSocket = path.join(os.tmpdir(), `arroba-tui-web-parity-visual-${process.pid}-${Date.now()}.sock`)
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const daemonId = `tui-web-parity-visual-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configRoot,
    XDG_STATE_HOME: stateRoot,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history-jsonl'),
    ARROBA_TEST_TUI: '1',
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
    await mkdir(path.join(configRoot, 'arroba'), { recursive: true })
    await mkdir(stateRoot, { recursive: true })
    await mkdir(evidenceDir, { recursive: true })
    await writeFile(path.join(configRoot, 'arroba', 'config.toml'), [
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
      schema: 'arroba.tui_web_parity_visual_session.v1',
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
      command: {
        control: `pnpm --filter @arroba/cli run tui-web-parity:visual-control -- --manifest ${options.manifestPath}`,
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

main().catch((error) => {
  console.error(`[tui-web-parity-visual] ${error.stack ?? error.message}`)
  process.exitCode = 1
})

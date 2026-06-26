#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 60_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    keepArtifactsOnFailure: true,
    preserveOnSuccess: true,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-drill.mjs [options]',
        '',
        'Runs a local meta-mode drill against a real kernel:',
        '- creates a regular session through arroba-shell',
        '- activates the focused regular agent with a /meta prompt',
        '- verifies meta-only runtime MCP tools over the real MCP server',
        '- prompts the owned agent through arroba.meta.run_command',
        '- verifies event inbox, turn overview/blob, and runtime interaction resolution',
        '',
        'Options:',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
        '  --discard-artifacts-on-failure',
        '  --preserve-on-success',
        '  --discard-artifacts-on-success',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 56500 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-drill] ${name}`)
  else console.log(`[metaagent-drill] ${name}`, JSON.stringify(details))
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

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Drill'], { cwd: root })
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (existing) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, env) {
  const scriptPath = path.join(workspace, 'wait.arroba')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await sleep(250)
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function launchRuntime(client, requests, sessionId, agentId, model, timeoutMs, pollMs) {
  const launched = unwrapVariant(
    await client.send(requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', model, 'low', agentId)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  let providerRun = launched.provider_run
  if (!providerRun?.id) throw new Error(`launch did not return provider run: ${JSON.stringify(launched)}`)
  const deadline = Date.now() + timeoutMs
  let last = providerRun
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
    if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
}

async function waitForAgentRuntime(client, requests, sessionId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    lastSession = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const providerRunId = lastSession.active_provider_run_id
    if (providerRunId) {
      const run = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
      if (run?.agent_instance_id === agentId || run?.agent_id === agentId) {
        let last = run
        while (Date.now() < deadline) {
          last = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
          if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
          if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
          await sleep(pollMs)
        }
        throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run for agent ${agentId}: ${JSON.stringify(lastSession)}`)
}

async function callRuntimeMcp(providerRun, method, params = {}) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: `${method}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method,
      params,
    }),
  })
  const text = await response.text()
  let json
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`runtime MCP response was not JSON (${response.status}): ${text}`)
  }
  if (!response.ok || json.error) throw new Error(`runtime MCP ${method} failed: ${text}`)
  return json.result
}

async function callRuntimeTool(providerRun, name, args = {}) {
  const result = await callRuntimeMcp(providerRun, 'tools/call', {
    name,
    arguments: args,
  })
  return {
    ok: !result.isError,
    payload: result.structuredContent,
    raw: result,
  }
}

async function listRuntimeToolNames(providerRun) {
  const result = await callRuntimeMcp(providerRun, 'tools/list')
  return (result.tools ?? []).map((tool) => tool.name)
}

async function waitForAgentIdle(client, requests, sessionId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded')
    const session = payload.session ?? payload
    last = session
    const promptState = session.prompt_states?.[agentId]
    const activeInteraction = (session.active_interactions ?? []).some((interaction) => interaction.agent_id === agentId)
    if (!promptState?.active_prompt && !activeInteraction) return session
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle\n${JSON.stringify(last, null, 2)}`)
}

async function waitForInteraction(client, requests, sessionId, agentId, title, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded')
    const session = payload.session ?? payload
    last = session
    const interaction = (session.active_interactions ?? [])
      .find((entry) => entry.agent_id === agentId && entry.title === title)
    if (interaction) return interaction
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for interaction ${title}\n${JSON.stringify(last, null, 2)}`)
}

async function waitForMetaEvent(providerRun, kind, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const events = await callRuntimeTool(providerRun, 'arroba.meta.list_events', { kind, limit: 10 })
    last = events.payload
    if (events.ok) {
      const event = (events.payload?.events ?? []).find((entry) => entry.kind === kind)
      if (event) return { events, event }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for metaagent event ${kind}\n${JSON.stringify(last, null, 2)}`)
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

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `metaagent-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    ARROBA_DEV_STUB_LARGE_OUTPUT_LINES: '2',
    ARROBA_DEV_STUB_LARGE_OUTPUT_LINE_BYTES: '64',
  }

  let daemon = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(scriptsDir, { recursive: true })
    await writeFile(
      path.join(workspace, 'README.md'),
      '# Metaagent basic fixture\n\nThe meta-mode drill reads this planning context.\n',
      'utf8',
    )
    await initGitWorktree(workspace)

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, env)
    log('daemon-ready', { kernelUrl })

    const successScript = path.join(scriptsDir, 'success.arroba')
    await writeFile(successScript, [
      'set provider dev-stub',
      'set model metaagent-drill-default',
      'session new $workspace as session',
      'mcp list',
      'skill list',
      'credential list',
      'slice list',
    ].join('\n'), 'utf8')
    const success = await run(process.execPath, [
      shellBin,
      'run',
      successScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      `workspace=${workspace}`,
    ], { env })
    if (success.code !== 0) {
      throw new Error(`success script failed\nstdout:\n${success.stdout}\nstderr:\n${success.stderr}`)
    }
    sessionId = success.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'success script did not bind session id', { stdout: success.stdout })
    log('shell-success-passed', { sessionId })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-drill-${Date.now()}`)), 'SessionAttached').attachment
    await client.subscribeToKernelEvents(sessionId, attachment.id)

    const sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const agents = sessionState.agents ?? []
    const metaagent = agents.find((agent) => agent.id === sessionState.focused_agent_id) ?? agents[0]
    assert(metaagent, 'session should contain a default regular agent', { agents })
    assert(!metaagent.meta_mode, 'default agent must start outside meta mode', metaagent)

    await client.send(requests.submitPromptRequest(
      sessionId,
      attachment.id,
      metaagent.id,
      '/meta Coordinate the basic metaagent runtime drill. Keep the task active while the harness verifies runtime MCP capabilities.',
      [],
    ))
    const metaRun = await waitForAgentRuntime(client, requests, sessionId, metaagent.id, options.timeoutMs, options.pollMs)
    assert(metaRun.execution_mode === 'plan', 'metaagent provider run must be forced to plan mode', { metaRun })
    const metaTools = await listRuntimeToolNames(metaRun)
    assert(metaTools.includes('arroba.meta.session_overview'), 'metaagent runtime MCP must expose meta tools', { metaTools })
    assert(metaTools.includes('arroba.read_artifact'), 'metaagent runtime MCP must expose read-only workspace tools', { metaTools })
    const directRead = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'README.md' })
    assert(directRead.ok, 'metaagent direct workspace read tool must be allowed for planning', directRead.payload)
    log('runtime-tool-exposure-passed')

    const workerSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'agent spawn worker large-output-drill',
    })
    assert(workerSpawn.ok, 'metaagent should spawn an owned regular worker', workerSpawn.payload)
    const worker = workerSpawn.payload?.response?.agent
    assert(worker?.id && !worker.meta_mode, 'worker should be a standard agent', workerSpawn.payload)
    const workerRun = await launchRuntime(client, requests, sessionId, worker.id, 'large-output-drill', options.timeoutMs, options.pollMs)
    const workerTools = await listRuntimeToolNames(workerRun)
    assert(!workerTools.includes('arroba.meta.session_overview'), 'standard agent runtime MCP must not expose meta tools', { workerTools })

    const workflowCreate = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'workflow new metaagent-drill-flow',
    })
    assert(workflowCreate.ok, 'metaagent should create an owned workflow', workflowCreate.payload)
    const workflowNode = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'workflow node add metaagent-drill-flow worker',
    })
    assert(workflowNode.ok, 'metaagent should add owned worker to workflow', workflowNode.payload)

    const overview = await callRuntimeTool(metaRun, 'arroba.meta.session_overview')
    assert(overview.ok, 'session_overview should succeed', overview.payload)
    assert((overview.payload?.agents?.owned ?? []).some((agent) => agent.id === worker.id), 'session_overview should include owned regular agent', overview.payload)

    const deniedSession = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'session new' })
    assert(!deniedSession.ok, 'session creation must be denied through run_command', deniedSession.payload)

    const promptResult = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'prompt worker "METAAGENT_DRILL_PROMPT_OK"',
    })
    assert(promptResult.ok, 'metaagent prompt command should be accepted for owned worker', promptResult.payload)
    await client.send(requests.appendNativeProviderOutputRequest(
      sessionId,
      attachment.id,
      workerRun.id,
      'provider_tool',
      JSON.stringify({
        tool: 'metaagent_drill_tool',
        status: 'completed',
        output: 'METAAGENT_DRILL_PROVIDER_OUTPUT',
      }),
      'metaagent-drill-output',
    ))
    await client.send(requests.completePromptRequest(sessionId))
    await waitForAgentIdle(client, requests, sessionId, worker.id, options.timeoutMs, options.pollMs)

    const { events, event } = await waitForMetaEvent(metaRun, 'agent.turn.completed', options.timeoutMs, options.pollMs)
    assert(events.ok, 'list_events should succeed', events.payload)
    const readEvent = await callRuntimeTool(metaRun, 'arroba.meta.read_event', { event_id: event.event_id })
    assert(readEvent.ok, 'read_event should return event detail', readEvent.payload)
    const ackEvent = await callRuntimeTool(metaRun, 'arroba.meta.ack_event', { event_id: event.event_id })
    assert(
      ackEvent.ok && (ackEvent.payload?.acked ?? []).some((entry) => entry.event_id === event.event_id),
      'ack_event should ack the selected event',
      ackEvent.payload,
    )

    const turnOverview = await callRuntimeTool(metaRun, 'arroba.meta.turn_overview', { agent_ref: 'worker', turns_back: 0 })
    assert(turnOverview.ok, 'turn_overview should succeed for owned regular agent', turnOverview.payload)
    const firstBlob = (turnOverview.payload?.turns ?? [])
      .flatMap((turn) => turn.items ?? [])
      .find((item) => item.blob_id)?.blob_id
    assert(firstBlob, 'turn_overview should expose at least one blob id', turnOverview.payload)
    const turnBlob = await callRuntimeTool(metaRun, 'arroba.meta.turn_blob', { blob_id: firstBlob })
    assert(turnBlob.ok, 'turn_blob should return selected blob detail', turnBlob.payload)

    const interactionTitle = `Metaagent Drill Permission ${Date.now()}`
    const interactionPromise = client.send(requests.requestNativeProviderInteractionRequest(
      sessionId,
      worker.id,
      `metaagent-drill-interaction-${Date.now()}`,
      interactionTitle,
      'The metaagent drill asks the metaagent to approve this owned regular-agent interaction.',
      30,
    ))
    const interaction = await waitForInteraction(client, requests, sessionId, worker.id, interactionTitle, options.timeoutMs, options.pollMs)
    const interactionEvents = await callRuntimeTool(metaRun, 'arroba.meta.list_events', { kind: 'runtime.interaction', limit: 10 })
    assert(interactionEvents.ok, 'runtime interaction events should be visible', interactionEvents.payload)
    const resolution = await callRuntimeTool(metaRun, 'arroba.meta.resolve_runtime_interaction', {
      interaction_id: interaction.id,
      choice_id: 'allow_once',
    })
    assert(resolution.ok, 'metaagent should resolve owned regular-agent interaction', resolution.payload)
    const interactionResult = unwrapVariant(await interactionPromise, 'NativeProviderInteractionResolved', 'RuntimeInteractionResolved')
    assert(
      interactionResult?.resolution?.choice_id === 'allow_once' || interactionResult?.choice_id === 'allow_once',
      'interaction should resolve with allow_once',
      interactionResult,
    )
    log('runtime-interaction-resolution-passed')

    const completeTask = await callRuntimeTool(metaRun, 'arroba.meta.complete_task', { summary: 'Basic meta-mode drill finished.' })
    assert(completeTask.ok && completeTask.payload?.status === 'completed', 'metaagent should mark its task completed', completeTask.payload)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-drill',
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      workerId: worker.id,
      eventId: event.event_id,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(kernelUrl, sessionId)
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'metaagent',
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
        scriptsDir,
      },
      log,
    })
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})

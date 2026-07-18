import { spawn } from 'node:child_process'
import { access, mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await (await import('node:fs/promises')).readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await (await import('node:fs/promises')).writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 240_000
const DEFAULT_POLL_MS = 1_000
const MAX_LOG_CHARS = 128_000

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    workspace: repoRoot,
    worktree: repoRoot,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    sliceRef: null,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--workspace') options.workspace = argv[++i]
    else if (arg === '--worktree') options.worktree = argv[++i]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--slice-ref') options.sliceRef = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-freeform-multi-agent-drill.mjs [options]',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    `  --workspace ${repoRoot}`,
    `  --worktree ${repoRoot}`,
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=opencode/gpt-5.2)',
    '  --slice-ref SLICE (run every drill agent on one existing slice)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
  ].join('\n'))
}

function makePorts() {
  const base = 56000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function appendOutput(buffer, chunk) {
  const next = buffer + chunk.toString()
  if (next.length <= MAX_LOG_CHARS) return next
  return next.slice(next.length - MAX_LOG_CHARS)
}

function tailLines(value, count = 80) {
  return value.split('\n').slice(-count).join('\n')
}

async function writeIfPresent(filePath, value) {
  if (!value) return
  await writeFile(filePath, value, 'utf8').catch(() => {})
}

function summarizeEvents(events) {
  const counts = new Map()
  for (const event of events) {
    const key = event.event ?? event.type ?? 'unknown'
    counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  if (provider === 'codex' && !options.model.includes('/')) return opencodeCodexModel(options.model)
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, worktree)), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error('daemon did not become ready')
}

function sessionStatePayload(response) {
  return unwrapVariant(response, 'SessionStateLoaded', 'SessionState')
}

function activityForAgent(state, agentId) {
  const activity = state?.agent_activity ?? state?.agentActivity ?? state?.session?.agent_activity ?? null
  if (Array.isArray(activity)) {
    return activity.find((entry) => entry?.agent_id === agentId || entry?.agentId === agentId) ?? null
  }
  return activity?.[agentId] ?? activity?.[String(agentId)] ?? null
}

function activityIsWorking(activity) {
  if (!activity) return false
  const status = String(activity.status ?? '').toLowerCase()
  const promptStatus = String(activity.prompt_status ?? activity.promptStatus ?? '').toLowerCase()
  const activePromptCount = Number(activity.active_prompt_count ?? activity.activePromptCount ?? 0)
  return activity.busy === true
    || activePromptCount > 0
    || Boolean(activity.active_turn ?? activity.activeTurn)
    || ['working', 'running', 'thinking', 'streaming'].includes(status)
    || (promptStatus !== '' && !['none', 'idle', 'completed', 'cancelled'].includes(promptStatus))
}

function completedOutlineTurn(outline, agentId, expectedPrompt) {
  const agent = (outline.agents ?? []).find((entry) => entry.agent_id === agentId)
  const turns = [...(agent?.turns ?? [])].reverse()
  for (const turn of turns) {
    const promptText = turn.user_prompt?.entry?.text ?? ''
    if (promptText.trim() !== expectedPrompt.trim()) continue
    const outputText = [
      ...(turn.entries ?? []).map((entry) => entry?.entry?.text ?? ''),
      turn.summary?.entry?.text ?? '',
    ].join('\n')
    const startedAtMs = Number(turn.started_at_ms ?? 0)
    const completedAtMs = Number(turn.completed_at_ms ?? 0)
    return {
      lifecycle: String(turn.lifecycle ?? '').toLowerCase(),
      startedAtMs,
      completedAtMs,
      durationMs: completedAtMs - startedAtMs,
      hasProviderResponse: outputText.trim().length > 0,
    }
  }
  return null
}

function naturalLifecyclePrompt(provider, index) {
  const subject = provider === 'claude-headless'
    ? 'why an interface should keep its running timer visible until the underlying operation has actually finished'
    : provider === 'codex'
      ? 'the difference between optimistically displaying a submitted message and declaring its operation complete'
      : 'why a page reload should preserve an active operation and its queued follow-up'
  return `In two short sentences, explain ${subject}. This is acceptance scenario ${index + 1}.`
}

async function waitForAuthoritativeLifecycles({
  client,
  requests,
  sessionId,
  attachmentId,
  agents,
  expectedPrompts,
  timeoutMs,
  pollMs,
}) {
  const deadline = Date.now() + timeoutMs
  const observations = new Map(agents.map((agent) => [agent.id, {
    sawWorking: false,
    sawIdleAfterWorking: false,
    activity: null,
    turn: null,
  }]))
  while (Date.now() < deadline) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const state = sessionStatePayload(await client.send(requests.getSessionStateRequest(sessionId)))
    for (const agent of agents) {
      const observation = observations.get(agent.id)
      const activity = activityForAgent(state, agent.id)
      observation.activity = activity
      if (activityIsWorking(activity)) {
        observation.sawWorking = true
        continue
      }
      if (!activity || !observation.sawWorking) continue
      observation.sawIdleAfterWorking = true
      const outline = unwrap(
        await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agent.id], 4)),
        'SessionHistoryOutline',
      )
      observation.turn = completedOutlineTurn(outline, agent.id, expectedPrompts.get(agent.id))
    }
    const complete = [...observations.values()].every((observation) => (
      observation.sawWorking
      && observation.sawIdleAfterWorking
      && observation.turn?.lifecycle === 'completed'
      && observation.turn.durationMs > 0
      && observation.turn.hasProviderResponse
    ))
    if (complete) {
      return agents.map((agent) => ({ agentId: agent.id, ...observations.get(agent.id) }))
    }
    await sleep(Math.min(pollMs, 250))
  }
  throw new Error(`timed out waiting for authoritative prompt lifecycles: ${JSON.stringify(Object.fromEntries(observations))}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  if (options.providers.length < 2) {
    throw new Error('freeform multi-agent drill requires at least two providers')
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-freeform-multi-agent-drill')
  const rootDir = path.join(repoRoot, '.artifacts', 'live-freeform-multi-agent-drill', nowStamp())
  let succeeded = false
  let failure = null
  let client = null
  let daemonChild = null
  let daemonStdout = ''
  let daemonStderr = ''
  let kernelUrl = options.kernel
  let sessionId = null
  let attachmentId = null
  let agents = []
  let listedAgents = []
  let processes = []
  let finalState = null
  let lifecycles = []
  const events = []

  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })

  let endSessionRequest = null
  try {
    await prepareDrillArtifacts(rootDir)
    const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
    const {
      attachToSessionRequest,
      createSessionRequest,
      getSessionStateRequest,
      listAgentsRequest,
      listProviderProcessesRequest,
      spawnAgentRequest,
      submitPromptRequest,
    } = requests
    endSessionRequest = requests.endSessionRequest

    if (options.spawnDaemon) {
      const ports = makePorts()
      kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
      const daemonBinary = await resolveBinary(
        path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
        path.join(repoRoot, 'apps/kernel/Cargo.toml'),
        'arroba-kernel',
      )
      daemonChild = spawn(daemonBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          ARROBA_KERNEL_PORT: String(ports.kernelPort),
          ARROBA_MCP_PORT: String(ports.mcpPort),
          ARROBA_OPENCODE_PORT: String(ports.opencodePort),
          ARROBA_CODEX_PORT: String(ports.codexPort),
          ARROBA_DAEMON_ID: `freeform-drill-${process.pid}-${Date.now()}`,
          ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
          ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
        },
        stdio: ['ignore', 'pipe', 'pipe'],
      })
      daemonChild.stdout.on('data', (chunk) => { daemonStdout = appendOutput(daemonStdout, chunk) })
      daemonChild.stderr.on('data', (chunk) => { daemonStderr = appendOutput(daemonStderr, chunk) })
      await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, options.workspace, options.worktree)
    }

    client = new LocalIpcClient(kernelUrl)
    const session = unwrap(await client.send(createSessionRequest(options.workspace, options.worktree)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `freeform-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    attachmentId = attachment.id
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const agent = unwrapVariant(
        await client.send(spawnAgentRequest(
          session.id,
          provider,
          `${provider}-${index + 1}`,
          modelForProvider(provider, options),
          options.worktree,
          'low',
          'plan',
          'required',
          undefined,
          undefined,
          options.sliceRef ?? undefined,
        )),
        'AgentSpawned',
      ).agent
      agents.push(agent)
    }

    const expectedPrompts = new Map(agents.map((agent, index) => [
      agent.id,
      naturalLifecyclePrompt(options.providers[index], index),
    ]))
    const lifecyclePromise = waitForAuthoritativeLifecycles({
      client,
      requests,
      sessionId: session.id,
      attachmentId: attachment.id,
      agents,
      expectedPrompts,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    for (let index = 0; index < agents.length; index += 1) {
      const agent = agents[index]
      const expectedPrompt = expectedPrompts.get(agent.id)
      await client.send(submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        expectedPrompt,
        [],
      ))
    }

    lifecycles = await lifecyclePromise
    finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    listedAgents = unwrapVariant(await client.send(listAgentsRequest(session.id)), 'AgentsListed').agents || []
    processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
    succeeded = true

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'freeform-multi-agent',
      kernelUrl,
      sessionId: session.id,
      providers: options.providers,
      model: options.model,
      agents: agents.map((agent) => ({
        id: agent.id,
        alias: agent.alias,
        provider: agent.provider,
      })),
      listedAgentCount: listedAgents.length,
      completionCount: lifecycles.length,
      lifecycles,
      providerProcesses: processes.map((process) => ({
        processId: process.process_id,
        provider: process.provider,
        pid: process.pid ?? null,
        ownerRunIds: process.owner_provider_run_ids || [],
      })),
      focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
    }, null, 2))
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client && sessionId && endSessionRequest) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client?.close().catch(() => {})
    await terminateChild(daemonChild)
    await writeIfPresent(path.join(rootDir, 'daemon.stdout.log'), daemonStdout)
    await writeIfPresent(path.join(rootDir, 'daemon.stderr.log'), daemonStderr)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      failure,
      metadata: {
        drill: 'live-freeform-multi-agent',
        kernelUrl,
        workspace: options.workspace,
        worktree: options.worktree,
        providers: options.providers,
        sliceRef: options.sliceRef,
        model: options.model,
        providerModels: options.providerModels,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        spawnDaemon: options.spawnDaemon,
        sessionId,
        attachmentId,
        agents: agents.map((agent) => ({
          id: agent.id,
          alias: agent.alias,
          provider: agent.provider,
        })),
        listedAgentCount: listedAgents.length,
        providerProcesses: processes.map((process) => ({
          processId: process.process_id,
          provider: process.provider,
          pid: process.pid ?? null,
          ownerRunIds: process.owner_provider_run_ids || [],
        })),
        focusedAgentId: finalState?.session?.focused_agent_id ?? finalState?.focused_agent_id ?? null,
        lifecycles,
        eventCount: events.length,
        eventCounts: summarizeEvents(events),
        recentEvents: events.slice(-20),
        daemonStdoutTail: tailLines(daemonStdout),
        daemonStderrTail: tailLines(daemonStderr),
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

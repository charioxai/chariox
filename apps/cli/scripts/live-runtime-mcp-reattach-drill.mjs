import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import {
  applyProviderModelOverride,
  parseProviderList,
  providerProfileMetadata,
  resolveProviderModel,
} from './lib/drill-provider-profiles.mjs'
import { waitForCondition } from './lib/drill-runtime-helpers.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['codex', 'opencode']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.providers = parseProviderList(argv[++i])
    else if (arg === '--providers') options.providers = parseProviderList(argv[++i])
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') applyProviderModelOverride(options.providerModels, argv[++i])
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-runtime-mcp-reattach-drill.mjs [options]',
    '',
    'Warms provider catalog endpoints before workspace live sync launch, then verifies Arroba runtime MCP tools survive detach/reattach.',
    '',
    'Options:',
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

function makePorts() {
  const base = 60000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

function modelForProvider(provider, options) {
  return resolveProviderModel(provider, {
    defaultModel: options.model,
    providerModels: options.providerModels,
  })
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

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

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
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

async function fileExists(filePath) {
  try {
    await access(filePath)
    return true
  } catch {
    return false
  }
}

async function waitForFile({ client, sessionId, attachmentId, filePath, expected, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    if (await fileExists(filePath)) {
      const actual = await readFile(filePath, 'utf8')
      if (actual === expected) return actual
      throw new Error(`unexpected content for ${filePath}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${filePath}`)
}

async function waitForAgentsIdle({ client, sessionId, attachmentId, agentIds, getSessionStateRequest, timeoutMs, pollMs }) {
  await waitForCondition({
    label: `agents to become idle: ${agentIds.join(', ')}`,
    timeoutMs,
    pollMs,
    observe: async () => {
      await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
      const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
      const session = state.session ?? state
      const promptStates = session.prompt_states ?? {}
      const agents = session.agents ?? []
      return agentIds.map((agentId) => {
        const agent = agents.find((candidate) => candidate.id === agentId) ?? null
        const promptState = promptStates[agentId] ?? {}
        return {
          agentId,
          found: Boolean(agent),
          agentState: agent?.state ?? null,
          isProcessing: agent?.is_processing ?? null,
          activePrompt: promptState.active_prompt ?? null,
          queuedPromptCount: (promptState.queued_prompts ?? []).length,
        }
      })
    },
    isReady: (observations) => observations.every((entry) =>
      entry.found &&
      !entry.isProcessing &&
      entry.agentState !== 'Working' &&
      entry.activePrompt == null &&
      entry.queuedPromptCount === 0
    ),
  })
}

async function countCompletedRuntimeToolCalls({ historyDir, agentId, toolNeedles }) {
  let count = 0
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.agent_id && entry.agent_id !== agentId) continue
      if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
      let update
      try {
        update = JSON.parse(entry.text)
      } catch {
        continue
      }
      const tool = String(update.tool ?? '').toLowerCase()
      if (update.status === 'completed' && toolNeedles.some((needle) => tool.includes(needle))) count += 1
    }
  }
  return count
}

async function waitForRuntimeToolCalls({ historyDir, agentId, toolNeedles, minCount, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const count = await countCompletedRuntimeToolCalls({ historyDir, agentId, toolNeedles })
    if (count >= minCount) return count
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${minCount} runtime MCP tool calls containing ${toolNeedles.join(', ')}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const runId = `${process.pid}-${Date.now()}`
  const ports = makePorts()
  const runtimeDir = path.join(cliRoot, `.tmp-live-runtime-mcp-reattach-drill-${runId}`)
  const rootDir = path.join(cliRoot, 'target', 'live-runtime-mcp-reattach-drill', runId)
  const workspace = path.join(rootDir, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  const historyDir = path.join(rootDir, 'history')
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`

  let succeeded = false
  let sessionId = null
  let client = null
  let daemonChild = null
  let LocalIpcClient = null
  let endSessionRequest = null
  let failure = null
  const startedAt = Date.now()
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(outputsDir, { recursive: true })
    await mkdir(runtimeDir, { recursive: true })
    await writeFile(path.join(workspace, 'seed.txt'), 'runtime-mcp-seed\n', 'utf8')

    const loaded = await loadCliModules(runtimeDir)
    LocalIpcClient = loaded.LocalIpcClient
    const requests = loaded.requests
    const {
      attachToSessionRequest,
      createSessionRequest,
      detachFromSessionRequest,
      getProviderCatalogRequest,
      getSessionStateRequest,
      spawnAgentRequest,
      submitPromptRequest,
    } = requests
    endSessionRequest = requests.endSessionRequest

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
        ARROBA_DAEMON_ID: `runtime-mcp-reattach-${runId}`,
        ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        ARROBA_SESSION_HISTORY_DIR: historyDir,
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace)
    client = new LocalIpcClient(kernelUrl)

    await client.send(getProviderCatalogRequest())

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    let attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `runtime-mcp-reattach-a-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const agents = []
    for (const provider of options.providers) {
      const agent = unwrapVariant(
        await client.send(spawnAgentRequest(
          session.id,
          provider,
          `${provider}-runtime-mcp-reattach`,
          modelForProvider(provider, options),
          workspace,
          'low',
        )),
        'AgentSpawned',
      ).agent
      agents.push({ provider, agent })
    }

    for (const { provider, agent } of agents) {
      const beforeCount = await countCompletedRuntimeToolCalls({
        historyDir,
        agentId: agent.id,
        toolNeedles: ['list_extensions', 'read_artifact'],
      })
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is a runtime MCP attachment drill.',
        'Do not use shell commands or direct filesystem tools.',
        'First call Arroba `list_extensions`.',
        'Then call Arroba `read_artifact` for `seed.txt` with domain text.',
        `Then call Arroba \`write_artifact\` for \`outputs/${provider}-before-reattach.txt\` with exactly \`${provider}:BEFORE_REATTACH_OK\\n\`.`,
        `Reply exactly ${provider.toUpperCase()}_BEFORE_REATTACH_DONE.`,
      ].join('\n'), []))
      await waitForFile({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        filePath: path.join(outputsDir, `${provider}-before-reattach.txt`),
        expected: `${provider}:BEFORE_REATTACH_OK\n`,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      await waitForRuntimeToolCalls({
        historyDir,
        agentId: agent.id,
        toolNeedles: ['list_extensions', 'read_artifact'],
        minCount: beforeCount + 2,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }

    await waitForAgentsIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentIds: agents.map(({ agent }) => agent.id),
      getSessionStateRequest,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })

    await client.send(detachFromSessionRequest(attachment.id))
    await client.close()
    client = new LocalIpcClient(kernelUrl)
    attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `runtime-mcp-reattach-b-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    for (const { provider, agent } of agents) {
      const beforeCount = await countCompletedRuntimeToolCalls({
        historyDir,
        agentId: agent.id,
        toolNeedles: ['list_extensions', 'read_artifact'],
      })
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is the after-reattach runtime MCP verification.',
        'Do not use shell commands or direct filesystem tools.',
        'First call Arroba `list_extensions` again.',
        `Then call Arroba \`read_artifact\` for \`outputs/${provider}-before-reattach.txt\` with domain text.`,
        `Then call Arroba \`write_artifact\` for \`outputs/${provider}-after-reattach.txt\` with exactly \`${provider}:AFTER_REATTACH_OK\\n\`.`,
        `Reply exactly ${provider.toUpperCase()}_AFTER_REATTACH_DONE.`,
      ].join('\n'), []))
      await waitForFile({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        filePath: path.join(outputsDir, `${provider}-after-reattach.txt`),
        expected: `${provider}:AFTER_REATTACH_OK\n`,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      await waitForRuntimeToolCalls({
        historyDir,
        agentId: agent.id,
        toolNeedles: ['list_extensions', 'read_artifact'],
        minCount: beforeCount + 2,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'runtime-mcp-reattach-drill',
      kernelUrl,
      workspace,
      providers: options.providers,
      durationMs: Date.now() - startedAt,
      agents: agents.map(({ provider, agent }) => ({
        provider,
        id: agent.id,
        alias: agent.alias,
      })),
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    if (sessionId && LocalIpcClient && endSessionRequest) {
      const cleanup = new LocalIpcClient(kernelUrl)
      await cleanup.send(endSessionRequest(sessionId)).catch(() => {})
      await cleanup.close().catch(() => {})
    }
    await terminateChild(daemonChild)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'runtime-mcp-reattach',
        ...providerProfileMetadata({
          providers: options.providers,
          defaultModel: options.model,
          providerModels: options.providerModels,
        }),
        providerModels: options.providerModels,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
        runtimeDir,
      },
      log: (name, details) => console.log(`[runtime-mcp-reattach-drill] ${name}`, JSON.stringify(details)),
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`runtime MCP reattach drill artifacts kept at ${rootDir}`)
      console.error(`runtime MCP reattach transient CLI modules kept at ${runtimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error?.stack || error?.message || String(error))
  process.exit(1)
})
